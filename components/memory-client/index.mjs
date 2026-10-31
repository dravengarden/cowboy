// A memory backend owns memory. This package owns bounded delivery from a
// native session to the backend named by the session-extension descriptor.
import { createHash, randomUUID } from "node:crypto";
import {
  lstat,
  mkdir,
  open,
  readdir,
  readFile,
  rename,
  unlink,
} from "node:fs/promises";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { userInfo } from "node:os";

export const MEMORY_TOOLS = [
  "memory_search",
  "memory_get",
  "memory_put",
  "memory_forget",
];
/** After a failed exchange, turns skip recall for this long. */
const BREAKER_MS = 30000;
/** The longest memory may hold a user turn. */
const BEGIN_BUDGET_MS = 5000;
/** The longest keeping a completed turn may hold its output. */
const FINISH_BUDGET_MS = 1000;
const UNAVAILABLE_NOTICE =
  "[Matrix memory unavailable for this turn. Continue normally and verify facts directly.]";
const id = (value) =>
  typeof value === "string" &&
  /^[A-Za-z0-9][A-Za-z0-9_.:@/-]{0,159}$/.test(value);
const hash = (value) =>
  createHash("sha256").update(JSON.stringify(value)).digest("hex");
export function publicText(value) {
  return (typeof value === "string" ? value : "")
    .replace(
      /-----BEGIN [^-]*PRIVATE KEY-----[\s\S]*?-----END [^-]*PRIVATE KEY-----/g,
      "[redacted private key]",
    )
    .replace(
      /(?:bearer\s+[a-z0-9._~+/-]{12,}|(?:sk-|gh[pousr]_|github_pat_)[a-z0-9_-]{16,}|(?:api[_-]?key|access[_-]?token|password|secret)\s*[:=]\s*["']?[^\s"',;]{6,})/gi,
      "[redacted credential]",
    )
    .slice(0, 12000);
}
async function privateDirectory(path) {
  await mkdir(path, { recursive: true, mode: 0o700 });
  const stat = await lstat(path);
  if (
    !stat.isDirectory() || stat.mode & 0o077 || stat.uid !== process.getuid()
  ) throw new Error("Matrix state must be private");
}
async function atomic(path, value) {
  const temporary = path + "." + randomUUID() + ".tmp";
  const file = await open(temporary, "wx", 0o600);
  try {
    await file.writeFile(JSON.stringify(value));
    await file.sync();
  } finally {
    await file.close();
  }
  await rename(temporary, path);
  const directory = await open(dirname(path), "r");
  try {
    await directory.sync();
  } finally {
    await directory.close();
  }
}

export async function matrixConfiguration(
  provider,
  { path = process.env.COWBOY_MATRIX_CONFIG } = {},
) {
  if (!["codex", "claude"].includes(provider)) {
    throw new Error("Unsupported Matrix Provider");
  }
  path ??= join(userInfo().homedir, ".config", "matrix", provider + ".json");
  if (!isAbsolute(path)) {
    throw new Error("Matrix configuration must be absolute");
  }
  let stat;
  try {
    stat = await lstat(path);
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
  if (
    !stat.isFile() || stat.mode & 0o077 || stat.uid !== process.getuid() ||
    stat.size > 65536
  ) throw new Error("Matrix configuration must be a private regular file");
  const config = JSON.parse(await readFile(path, "utf8"));
  const url = new URL(config.endpoint);
  if (
    config.schema !== 1 || config.provider !== provider ||
    !Array.isArray(config.projects) ||
    !isAbsolute(config.state_dir ?? "") || typeof config.token !== "string" ||
    !/^[a-zA-Z0-9_-]{32,256}$/.test(config.token) ||
    url.username || url.password || url.search || url.hash ||
    url.pathname !== "/" ||
    !(url.protocol === "https:" ||
      url.protocol === "http:" && ["127.0.0.1", "[::1]"].includes(url.hostname))
  ) throw new Error("Invalid Matrix configuration");
  return config;
}

const PROTOCOLS = ["matrix_http_v1"];

/** Memory is never required: an agent keeps working through every failure
 * below. Operators read the reason; it never reaches model context. */
function unavailable(stage, error) {
  process.stderr.write(
    "Matrix " + stage + " unavailable: " + String(error?.message ?? error) +
      "\n",
  );
}
const BINDING_DIRECTORY = ["cowboy", "memory"];

async function privateFile(path, limit) {
  let stat;
  try {
    stat = await lstat(path);
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
  if (
    !stat.isFile() || stat.mode & 0o077 || stat.uid !== process.getuid() ||
    stat.size > limit
  ) throw new Error("Session extension files must be private regular files");
  return await readFile(path, "utf8");
}

/** The session's extension descriptor written by the runtime Machine, or
 * `null` when the Machine predates session extensions. */
const DISABLED = Object.freeze({ schema: 1, memory: null, calls: null });

export async function sessionExtensions(
  { path = process.env.COWBOY_SESSION_EXTENSIONS } = {},
) {
  if (!path) return null;
  // The Machine could not resolve this session's extensions: run without.
  if (path === "disabled") return DISABLED;
  // A descriptor that is missing, unsafe or malformed disables extensions
  // for this process; it never falls back to a host enrollment.
  try {
    if (!isAbsolute(path)) throw new Error("descriptor path is not absolute");
    const text = await privateFile(path, 65536);
    if (text === null) throw new Error("descriptor is missing");
    const value = JSON.parse(text);
    if (value?.schema !== 1) throw new Error("unsupported descriptor schema");
    return value;
  } catch (error) {
    process.stderr.write(
      "Session extensions disabled: " + String(error?.message ?? error) + "\n",
    );
    return DISABLED;
  }
}

function configHome() {
  return process.env.XDG_CONFIG_HOME && isAbsolute(process.env.XDG_CONFIG_HOME)
    ? process.env.XDG_CONFIG_HOME
    : join(userInfo().homedir, ".config");
}

/** The session's Machine relay. Its endpoint and token are scoped to this
 * session, and the Controller supplies the identity, so no project mapping
 * or backend credential exists here. */
function relayConfiguration(provider, relay) {
  if (!relay) return null;
  const url = new URL(relay.endpoint);
  if (
    url.protocol !== "http:" || url.hostname !== "127.0.0.1" ||
    typeof relay.token !== "string" || !isAbsolute(relay.state_dir ?? "")
  ) throw new Error("Invalid memory relay");
  return {
    provider,
    relay: true,
    endpoint: relay.endpoint,
    token: relay.token,
    state_dir: relay.state_dir,
    projects: [],
  };
}

/** Memory for one Provider session. With a descriptor, the installed
 * backend and the session's switches decide; its Machine-private binding
 * lives at $XDG_CONFIG_HOME/cowboy/memory/<plugin>/<provider>.json, and the
 * Matrix Plugin also accepts the legacy ~/.config/matrix enrollment. Without
 * a descriptor the legacy enrollment applies with every capability on. */
export async function memoryConfiguration(provider, options) {
  try {
    return await resolveMemory(provider, options);
  } catch (error) {
    unavailable("configuration", error);
    return null;
  }
}

async function resolveMemory(provider, { extensions, legacyPath } = {}) {
  const descriptor = extensions === undefined
    ? await sessionExtensions()
    : extensions;
  if (descriptor === null) {
    const config = await matrixConfiguration(provider, { path: legacyPath });
    return config && {
      ...config,
      server: "matrix",
      tools: [
        ...MEMORY_TOOLS,
        "memory_read",
        "memory_execute",
        "memory_receipt",
      ],
      enableTools: true,
      enableRecall: true,
      observe: true,
    };
  }
  const memory = descriptor.memory;
  if (!memory || !PROTOCOLS.includes(memory.protocol)) return null;
  const plugin = memory.plugin?.id;
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(plugin ?? "")) {
    throw new Error("Invalid memory backend identity");
  }
  const enableTools = memory.enable_tools === true;
  const enableRecall = memory.enable_recall === true;
  const observe = memory.observe === true;
  if (!enableTools && !enableRecall && !observe) return null;
  // A Machine with a local backend uses it directly; every other Machine
  // reaches the backend through its Cowboy connection.
  let config = await matrixConfiguration(provider, {
    path: join(configHome(), ...BINDING_DIRECTORY, plugin, provider + ".json"),
  });
  if (!config && plugin === "matrix") {
    config = await matrixConfiguration(provider, { path: legacyPath });
  }
  config ??= relayConfiguration(provider, memory.relay);
  if (!config) return null;
  const tools = Array.isArray(memory.tools) ? memory.tools : [];
  if (!tools.every((tool) => /^[a-z][a-z0-9_]{0,63}$/.test(tool))) {
    throw new Error("Invalid memory backend tools");
  }
  return {
    ...config,
    server: plugin,
    tools,
    enableTools: enableTools && tools.length > 0,
    enableRecall,
    observe,
  };
}

/** Agent-facing call guidance: the installed call profiles' text with the
 * calling Provider's peer filled in, else the Provider's built-in text. */
export function callGuidance(extensions, { peer, peerCommand, fallback }) {
  const profiles = extensions?.calls?.profiles;
  if (!Array.isArray(profiles) || profiles.length === 0) return fallback;
  return profiles
    .map((profile) => String(profile.guidance ?? ""))
    .filter((text) => text.trim())
    .map((text) =>
      text.replaceAll("{{peer}}", peer).replaceAll(
        "{{peer_command}}",
        peerCommand,
      )
    )
    .join("\n\n") || fallback;
}

/** Exact native MCP names a Provider may allow for this configuration. */
export function memoryToolNames(config) {
  return config?.enableTools
    ? config.tools.map((tool) => `mcp__${config.server}__${tool}`)
    : [];
}

/** The relay's Controller-derived identity replaces this placeholder. */
const RELAY_BINDING = { project: "relay", machine: "relay", session: "relay" };

function selectBinding(config, descriptor, cwd, session) {
  if (config.relay) return RELAY_BINDING;
  const binding = descriptor?.binding;
  const matches = config.projects.filter((project) =>
    binding
      ? project.workspace === binding.workspace.id &&
        project.machine === binding.environment.machine_id
      : project.path && resolve(project.path) === resolve(cwd)
  );
  // A workspace is one project on every Machine. An unmapped one uses its own
  // id; the service still checks the project against this client's grant.
  const elsewhere = binding
    ? [
      ...new Set(
        config.projects
          .filter((project) => project.workspace === binding.workspace.id)
          .map((project) => project.project),
      ),
    ]
    : [];
  const selected = {
    project: matches.length === 1
      ? matches[0].project
      : elsewhere.length === 1
      ? elsewhere[0]
      : binding?.workspace.id,
    machine: matches.length === 1
      ? matches[0].machine
      : binding?.environment.machine_id,
    session: binding?.id ?? session,
  };
  if (!Object.values(selected).every(id)) {
    throw new Error("Matrix project mapping is missing or ambiguous");
  }
  return selected;
}

export class MatrixClient {
  /** `null` when this session has no usable memory, for any reason. */
  static async open(config, descriptor, options) {
    if (!config) return null;
    try {
      return await MatrixClient.#open(config, descriptor, options);
    } catch (error) {
      unavailable("session", error);
      return null;
    }
  }
  static async #open(
    config,
    descriptor,
    { cwd = process.cwd(), session = randomUUID() } = {},
  ) {
    const selected = selectBinding(config, descriptor, cwd, session);
    await privateDirectory(config.state_dir);
    const directory = join(config.state_dir, config.provider);
    await privateDirectory(directory);
    const client = new MatrixClient(config, selected, directory);
    // Delivery also probes: a success closes the recall breaker.
    client.timer = setInterval(() => {
      void client.flush();
    }, 10000);
    client.timer.unref();
    return client;
  }
  constructor(config, binding, directory) {
    this.config = config;
    this.binding = binding;
    this.directory = directory;
    this.events = [];
    this.bytes = 0;
    this.turn = randomUUID();
    this.delivery = Promise.resolve();
    this.inFlight = new Set();
    this.unavailableUntil = 0;
  }
  async request(action, payload, binding = this.binding) {
    try {
      const reply = await this.#exchange(action, payload, binding);
      this.unavailableUntil = 0;
      return reply;
    } catch (error) {
      // A refusal is an answer from a healthy service; only silence opens
      // the breaker, so an outage costs one wait instead of one per turn.
      if (!error.refused) this.unavailableUntil = Date.now() + BREAKER_MS;
      throw error;
    }
  }
  async #exchange(action, payload, binding) {
    const response = await fetch(
      new URL("/v1/" + action, this.config.endpoint),
      {
        method: "POST",
        redirect: "error",
        signal: AbortSignal.timeout(this.config.relay ? 4000 : 2000),
        headers: {
          "Content-Type": "application/json",
          Authorization: "Bearer " + this.config.token,
        },
        body: JSON.stringify({ binding, payload }),
      },
    );
    let size = 0;
    const chunks = [];
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > 524288) throw new Error("Matrix response exceeds limit");
      chunks.push(Buffer.from(chunk));
    }
    if (!response.ok) {
      const error = new Error(
        "Matrix request refused (" + response.status + ")",
      );
      // Rate and authentication answers may pass later.
      error.refused = response.status < 500 &&
        ![401, 408, 429].includes(response.status);
      throw error;
    }
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  }
  mcp() {
    return {
      type: "http",
      url: new URL("/mcp", this.config.endpoint).href,
      headers: {
        Authorization: "Bearer " + this.config.token,
        "X-Matrix-Binding": JSON.stringify(this.binding),
      },
    };
  }
  /** Bounded so a stalled service can delay a turn only briefly; no failure
   * leaves here. Delivery failures keep their evidence in the outbox. */
  async begin(prompt) {
    const notice = this.recall ? UNAVAILABLE_NOTICE : "";
    let timer;
    const bounded = new Promise((resolve) => {
      timer = setTimeout(() => {
        // Whatever stalled, the next turns must not wait for it again.
        this.unavailableUntil = Date.now() + BREAKER_MS;
        resolve(notice);
      }, BEGIN_BUDGET_MS);
    });
    try {
      return await Promise.race([this.#begin(prompt), bounded]);
    } catch (error) {
      unavailable("recall", error);
      return notice;
    } finally {
      clearTimeout(timer);
    }
  }
  async finish() {
    let timer;
    const bounded = new Promise((resolve) => {
      timer = setTimeout(resolve, FINISH_BUDGET_MS);
    });
    try {
      // Output frames wait here: a stalled disk may not hold them.
      await Promise.race([this.#finish(), bounded]);
    } catch (error) {
      unavailable("delivery", error);
    } finally {
      clearTimeout(timer);
    }
  }
  get recall() {
    return this.config.enableRecall !== false;
  }
  get tools() {
    return this.config.enableTools !== false;
  }
  get observe() {
    return this.config.observe !== false;
  }
  add(role, value) {
    if (!this.observe) return;
    const text = publicText(value);
    if (
      !text.trim() || !["user", "assistant", "tool"].includes(role) ||
      text.includes("[Matrix memory")
    ) return;
    // Leave headroom for JSON framing and Chinese UTF-8; do not retain images/reasoning.
    const bytes = Buffer.byteLength(text);
    if (this.events.length >= 200 || this.bytes + bytes > 200000) return;
    this.events.push({ id: "e" + this.events.length, role, text });
    this.bytes += bytes;
  }
  async enqueue(payload, reserved = false) {
    const names = (await readdir(this.directory)).filter((name) =>
      /^[a-f0-9]{64}\.json$/.test(name)
    );
    if (names.length >= 512) throw new Error("Matrix delivery queue is full");
    const value = { binding: this.binding, payload };
    const path = join(this.directory, hash(value) + ".json");
    if (reserved) this.inFlight.add(path);
    try {
      await atomic(path, value);
    } catch (error) {
      this.inFlight.delete(path);
      throw error;
    }
    return path;
  }
  flush() {
    // One delivery loop per process; concurrent processes use idempotent observations.
    this.delivery = this.delivery.then(async () => {
      const names = (await readdir(this.directory)).filter((name) =>
        /^[a-f0-9]{64}\.json$/.test(name)
      ).slice(0, 8);
      for (const name of names) {
        const path = join(this.directory, name);
        if (this.inFlight.has(path)) continue;
        try {
          const stat = await lstat(path);
          if (!stat.isFile() || stat.mode & 0o077 || stat.size > 524288) {
            throw new Error("Invalid Matrix outbox");
          }
          const value = JSON.parse(await readFile(path, "utf8"));
          if (this.inFlight.has(path)) continue;
          await this.request("observe", value.payload, value.binding);
          await unlink(path).catch((error) => {
            if (error.code !== "ENOENT") throw error;
          });
        } catch (error) {
          if (error.code === "ENOENT") continue;
          break;
        }
      }
    }).catch(() => {});
    return this.delivery;
  }
  async #begin(prompt) {
    // Steering may arrive in an existing turn. Retain the previous segment first.
    await this.finish();
    this.add("user", prompt);
    // Without recall or tools nothing is injected; the user turn stays in
    // the segment the turn's completion delivers for learning.
    if (!this.recall && !this.tools) return "";
    // Local to this turn: a late completion can reach no other turn.
    let context = this.recall ? UNAVAILABLE_NOTICE : "";
    if (!this.events.length) {
      // A backend that recalls without observing still answers context.
      if (
        !this.recall || !publicText(prompt).trim() ||
        Date.now() < this.unavailableUntil
      ) return context;
      try {
        const found = await this.request("context", {
          query: publicText(prompt),
          budget: 6000,
        });
        context = found.text;
      } catch {
        /* Keep the explicit unavailable notice. */
      }
      return context;
    }
    const evidence = {
      turn: this.turn + "-user",
      events: this.events,
      learn: false,
    };
    // Known down: keep the evidence for delivery and do not wait for a reply.
    if (Date.now() < this.unavailableUntil) {
      await this.enqueue(evidence);
      return context;
    }
    // Reserve before publishing the file so a concurrent background flush
    // cannot send the same current-user observation. Durable evidence remains
    // available to a restarted process if this request never gets a receipt.
    const path = await this.enqueue(evidence, true);
    try {
      const receipt = await this.request("observe", evidence);
      await unlink(path).catch((error) => {
        if (error.code !== "ENOENT") throw error;
      });
      const found = this.recall
        ? await this.request("context", {
          query: publicText(prompt),
          budget: 6000,
        })
        : { text: "" };
      context = found.text +
        (this.tools
          ? "\nCurrent user evidence for memory_put: " +
            receipt.events.join(", ") +
            ". Cite an exact supporting excerpt; automatic learning also runs after the turn.\n"
          : "");
    } catch {
      /* Preserve unacknowledged evidence and make absence explicit. */
    } finally {
      this.inFlight.delete(path);
    }
    void this.flush();
    return context;
  }
  async #finish() {
    const events = this.events;
    const turn = this.turn;
    this.events = [];
    this.bytes = 0;
    this.turn = randomUUID();
    if (events.length) await this.enqueue({ turn, events });
    void this.flush();
  }
  async close() {
    clearInterval(this.timer);
    await this.finish();
  }
}
