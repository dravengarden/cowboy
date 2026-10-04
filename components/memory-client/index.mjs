// Matrix owns memory. This package owns bounded delivery from a native session.
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

export class MatrixClient {
  static async open(
    config,
    descriptor,
    { cwd = process.cwd(), session = randomUUID() } = {},
  ) {
    if (!config) return null;
    const binding = descriptor?.binding;
    const matches = config.projects.filter((project) =>
      binding
        ? project.workspace === binding.workspace.id &&
          project.machine === binding.environment.machine_id
        : project.path && resolve(project.path) === resolve(cwd)
    );
    if (matches.length !== 1) {
      throw new Error("Matrix project mapping is missing or ambiguous");
    }
    const project = matches[0];
    const selected = {
      project: project.project,
      machine: project.machine,
      session: binding?.id ?? session,
    };
    if (!Object.values(selected).every(id)) {
      throw new Error("Invalid Matrix session binding");
    }
    await privateDirectory(config.state_dir);
    const directory = join(config.state_dir, config.provider);
    await privateDirectory(directory);
    const client = new MatrixClient(config, selected, directory);
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
    this.context = "";
    this.turn = randomUUID();
    this.delivery = Promise.resolve();
    this.inFlight = new Set();
  }
  async request(action, payload, binding = this.binding) {
    const response = await fetch(
      new URL("/v1/" + action, this.config.endpoint),
      {
        method: "POST",
        redirect: "error",
        signal: AbortSignal.timeout(2000),
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
      throw new Error("Matrix request refused (" + response.status + ")");
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
  add(role, value) {
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
  async begin(prompt) {
    // Steering may arrive in an existing turn. Retain the previous segment first.
    await this.finish();
    this.add("user", prompt);
    this.context =
      "[Matrix memory unavailable; native memory remains disabled. Verify facts directly.]";
    if (!this.events.length) return this.context;
    const evidence = {
      turn: this.turn + "-user",
      events: this.events,
      learn: false,
    };
    // Reserve before publishing the file so a concurrent background flush
    // cannot send the same current-user observation. Durable evidence remains
    // available to a restarted process if this request never gets a receipt.
    const path = await this.enqueue(evidence, true);
    try {
      const receipt = await this.request("observe", evidence);
      await unlink(path).catch((error) => {
        if (error.code !== "ENOENT") throw error;
      });
      const found = await this.request("context", {
        query: publicText(prompt),
        budget: 6000,
      });
      this.context = found.text + "\nCurrent user evidence for memory_put: " +
        receipt.events.join(", ") +
        ". Cite an exact supporting excerpt; automatic learning also runs after the turn.\n";
    } catch {
      /* Preserve unacknowledged evidence and make absence explicit. */
    } finally {
      this.inFlight.delete(path);
    }
    void this.flush();
    return this.context;
  }
  async finish() {
    if (this.events.length) {
      await this.enqueue({ turn: this.turn, events: this.events });
    }
    this.events = [];
    this.bytes = 0;
    this.turn = randomUUID();
    void this.flush();
  }
  async close() {
    clearInterval(this.timer);
    await this.finish();
  }
}
