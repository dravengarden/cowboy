// Signed Codex Level 2 per-request observation binding (`codex-otlp-http-v1`).
// Consumes the worker's private loopback OTLP HTTP logs binding ONLY when no
// user, project, system, managed, or CLI configuration already declares an
// OTel exporter or headers_helper policy. Fails closed on any probe error,
// remote/split execution placement, environment-dumping NODE_DEBUG, FIFO or
// oversized config, or TOML parse error, and passes the auth header
// exclusively through `otel.headers_helper` (never in `-c` argv or on disk).
import { spawn } from "node:child_process";
import {
  closeSync,
  constants as fsConstants,
  existsSync,
  fstatSync,
  openSync,
  readSync,
} from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";

export const CODEX_OBSERVATION_ENV = "COWBOY_PRIVATE_CODEX_REQUEST_OBSERVATION";
export const CODEX_OBSERVATION_HEADER = "x-cowboy-request-observation";
/** Port 0 refuses connections locally so the config-only probe can never rotate credentials. */
export const REFRESH_REFUSED_URL = "http://127.0.0.1:0/cowboy-refresh-refused";
const MAX_CONFIG_BYTES = 256 * 1024;
const OTEL_POLICY_KEYS = new Set([
  "exporter",
  "headers_helper",
  "trace_exporter",
  "metrics_exporter",
]);

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/**
 * Node's `NODE_DEBUG` / `NODE_DEBUG_NATIVE` (e.g. `child_process` or `*`) can
 * dump spawned environment variables to stderr, which Cowboy journals to disk.
 * Fail closed whenever any Node debug channel is enabled.
 */
export function hasEnvDumpingNodeDebug(env = process.env) {
  if (!env || typeof env !== "object") return false;
  return (
    String(env.NODE_DEBUG ?? "").trim() !== "" || String(env.NODE_DEBUG_NATIVE ?? "").trim() !== ""
  );
}

/** Strictly parse and validate the worker's private observation descriptor. */
export function parseObservationBinding(raw) {
  if (typeof raw !== "string" || raw.length === 0 || raw.length > 4096) return null;
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isPlainObject(parsed)) return null;
  if (parsed.schema !== 1) return null;
  if (parsed.header_name !== CODEX_OBSERVATION_HEADER) return null;
  const value = parsed.header_value;
  if (typeof value !== "string" || value.length === 0 || value.length > 256) return null;
  if (!/^[\x21-\x7e]+$/.test(value)) return null;
  const endpoint = parsed.logs_endpoint;
  if (typeof endpoint !== "string") return null;
  let url;
  try {
    url = new URL(endpoint);
  } catch {
    return null;
  }
  if (
    url.protocol !== "http:" ||
    url.hostname !== "127.0.0.1" ||
    !url.port ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.pathname !== "/v1/logs"
  ) {
    return null;
  }
  return {
    schema: 1,
    logs_endpoint: url.href,
    header_name: CODEX_OBSERVATION_HEADER,
    header_value: value,
  };
}

function isOtelPolicyPath(segments) {
  if (segments.length < 2 || segments[0] !== "otel") return false;
  return OTEL_POLICY_KEYS.has(segments[1]);
}

function createTomlCursor(text) {
  let pos = 0;
  const len = text.length;

  const peek = (offset = 0) => (pos + offset < len ? text[pos + offset] : "");

  const skipSpaces = () => {
    while (pos < len && (text[pos] === " " || text[pos] === "\t")) pos += 1;
  };

  const skipCommentToEol = () => {
    while (pos < len && text[pos] !== "\n" && text[pos] !== "\r") pos += 1;
  };

  const skipWsAndComments = () => {
    while (pos < len) {
      const ch = text[pos];
      if (ch === " " || ch === "\t" || ch === "\n" || ch === "\r") {
        pos += 1;
      } else if (ch === "#") {
        skipCommentToEol();
      } else {
        break;
      }
    }
  };

  const readBasicString = () => {
    pos += 1;
    let out = "";
    while (pos < len) {
      const ch = text[pos];
      if (ch === '"') {
        pos += 1;
        return out;
      }
      if (ch === "\n" || ch === "\r") throw new Error("unterminated basic string");
      if (ch === "\\") {
        pos += 1;
        if (pos >= len) throw new Error("unterminated escape");
        const esc = text[pos++];
        if (esc === '"' || esc === "\\") out += esc;
        else if (esc === "b") out += "\b";
        else if (esc === "t") out += "\t";
        else if (esc === "n") out += "\n";
        else if (esc === "f") out += "\f";
        else if (esc === "r") out += "\r";
        else if (esc === "u" || esc === "U") {
          const digits = esc === "u" ? 4 : 8;
          const hex = text.slice(pos, pos + digits);
          if (!/^[0-9a-fA-F]+$/.test(hex) || hex.length !== digits) {
            throw new Error("invalid unicode escape");
          }
          out += String.fromCodePoint(Number.parseInt(hex, 16));
          pos += digits;
        } else {
          throw new Error("invalid escape");
        }
        continue;
      }
      out += ch;
      pos += 1;
    }
    throw new Error("unterminated basic string");
  };

  const readLiteralString = () => {
    pos += 1;
    const start = pos;
    while (pos < len) {
      const ch = text[pos];
      if (ch === "'") {
        const out = text.slice(start, pos);
        pos += 1;
        return out;
      }
      if (ch === "\n" || ch === "\r") throw new Error("unterminated literal string");
      pos += 1;
    }
    throw new Error("unterminated literal string");
  };

  const skipMultilineBasic = () => {
    pos += 3;
    while (pos < len) {
      if (text[pos] === "\\") {
        pos += 2;
        continue;
      }
      if (text.startsWith('"""', pos)) {
        pos += 3;
        while (text[pos] === '"' && pos < len) pos += 1;
        return;
      }
      pos += 1;
    }
    throw new Error("unterminated multiline basic string");
  };

  const skipMultilineLiteral = () => {
    pos += 3;
    while (pos < len) {
      if (text.startsWith("'''", pos)) {
        pos += 3;
        while (text[pos] === "'" && pos < len) pos += 1;
        return;
      }
      pos += 1;
    }
    throw new Error("unterminated multiline literal string");
  };

  const readKeySegment = () => {
    skipSpaces();
    if (pos >= len) throw new Error("expected key");
    const ch = text[pos];
    if (ch === '"') return readBasicString();
    if (ch === "'") return readLiteralString();
    const start = pos;
    while (pos < len && /[A-Za-z0-9_-]/.test(text[pos])) pos += 1;
    if (pos === start) throw new Error("invalid bare key");
    return text.slice(start, pos);
  };

  const readKeyPath = () => {
    const segments = [readKeySegment()];
    for (;;) {
      skipSpaces();
      if (peek() !== ".") break;
      pos += 1;
      segments.push(readKeySegment());
    }
    return segments;
  };

  const scanValue = (prefixSegments, onKeyPath, depth = 0) => {
    if (depth > 32) throw new Error("nesting too deep");
    skipSpaces();
    if (pos >= len) throw new Error("expected value");
    if (text.startsWith('"""', pos)) {
      skipMultilineBasic();
      return;
    }
    if (text.startsWith("'''", pos)) {
      skipMultilineLiteral();
      return;
    }
    const ch = text[pos];
    if (ch === '"') {
      readBasicString();
      return;
    }
    if (ch === "'") {
      readLiteralString();
      return;
    }
    if (ch === "{") {
      pos += 1;
      skipWsAndComments();
      if (peek() === "}") {
        pos += 1;
        return;
      }
      for (;;) {
        skipWsAndComments();
        if (peek() === "}") {
          pos += 1;
          return;
        }
        const subKey = readKeyPath();
        skipSpaces();
        if (peek() !== "=") throw new Error("expected '=' in inline table");
        pos += 1;
        const fullPath = [...prefixSegments, ...subKey];
        onKeyPath(fullPath);
        scanValue(fullPath, onKeyPath, depth + 1);
        skipWsAndComments();
        if (peek() === ",") {
          pos += 1;
          continue;
        }
        if (peek() === "}") {
          pos += 1;
          return;
        }
        throw new Error("expected ',' or '}' in inline table");
      }
    }
    if (ch === "[") {
      pos += 1;
      for (;;) {
        skipWsAndComments();
        if (peek() === "]") {
          pos += 1;
          return;
        }
        scanValue([], () => {}, depth + 1);
        skipWsAndComments();
        if (peek() === ",") {
          pos += 1;
          continue;
        }
        if (peek() === "]") {
          pos += 1;
          return;
        }
        throw new Error("expected ',' or ']' in array");
      }
    }
    const start = pos;
    while (pos < len && !/[\s#,}\]]/.test(text[pos])) pos += 1;
    if (pos === start) throw new Error("empty scalar");
    const raw = text.slice(start, pos);
    // Allow only legal TOML bare scalars: booleans, integers, floats, inf/nan, or RFC3339 date/time.
    const validScalar =
      raw === "true" ||
      raw === "false" ||
      /^[+-]?(?:inf|nan)$/.test(raw) ||
      /^0x[0-9a-fA-F](?:_?[0-9a-fA-F])*$/.test(raw) ||
      /^0o[0-7](?:_?[0-7])*$/.test(raw) ||
      /^0b[01](?:_?[01])*$/.test(raw) ||
      /^[+-]?(?:0|[1-9](?:_?[0-9])*)(?:\.[0-9](?:_?[0-9])*)?(?:[eE][+-]?[0-9](?:_?[0-9])*)?$/.test(
        raw,
      ) ||
      /^\d{4}-\d{2}-\d{2}(?:[Tt ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:\d{2})?)?$/.test(
        raw,
      ) ||
      /^\d{2}:\d{2}:\d{2}(?:\.\d+)?$/.test(raw);
    if (!validScalar) throw new Error(`invalid TOML scalar: ${raw}`);
  };

  return {
    eof: () => pos >= len,
    peek,
    advance: (n = 1) => {
      pos += n;
    },
    skipSpaces,
    skipCommentToEol,
    skipWsAndComments,
    readKeyPath,
    scanValue,
  };
}

/**
 * Structurally inspect TOML text for any `[otel]` / `["otel"]` / `otel = { ... }`
 * / `otel.exporter` / `otel.headers_helper` policy declaration.
 * Fails closed (`true`) on any malformed TOML so an unreadable opt-out can
 * never be bypassed.
 */
export function tomlHasOtelPolicy(text) {
  if (typeof text !== "string" || !text) return false;
  if (Buffer.byteLength(text, "utf8") > MAX_CONFIG_BYTES) return true;
  let found = false;
  const definedTables = new Set();
  const definedKeys = new Set();
  const pathKey = (segments) => JSON.stringify(segments);
  const checkPath = (segments) => {
    if (isOtelPolicyPath(segments)) found = true;
  };
  try {
    const cursor = createTomlCursor(text);
    let currentTable = [];
    let currentArrayIndex = -1;
    while (!cursor.eof()) {
      cursor.skipWsAndComments();
      if (cursor.eof()) break;
      if (cursor.peek() === "[") {
        const isArrayTable = cursor.peek(1) === "[";
        cursor.advance(isArrayTable ? 2 : 1);
        const header = cursor.readKeyPath();
        cursor.skipSpaces();
        if (isArrayTable) {
          if (cursor.peek() !== "]" || cursor.peek(1) !== "]") {
            throw new Error("unclosed array table header");
          }
          cursor.advance(2);
          currentArrayIndex += 1;
        } else {
          if (cursor.peek() !== "]") throw new Error("unclosed table header");
          cursor.advance(1);
          const tk = pathKey(header);
          if (definedTables.has(tk) || definedKeys.has(tk)) {
            throw new Error(`duplicate table definition: ${tk}`);
          }
          definedTables.add(tk);
        }
        currentTable = header;
        checkPath(currentTable);
        if (found) return true;
        cursor.skipSpaces();
        if (cursor.peek() === "#") cursor.skipCommentToEol();
        if (!cursor.eof() && cursor.peek() !== "\n" && cursor.peek() !== "\r") {
          throw new Error("trailing tokens after table header");
        }
        continue;
      }
      const key = cursor.readKeyPath();
      cursor.skipSpaces();
      if (cursor.peek() !== "=") throw new Error("expected '=' after key");
      cursor.advance(1);
      const fullPath = [...currentTable, ...key];
      const scopedKey = `${currentArrayIndex}:${pathKey(fullPath)}`;
      if (
        definedKeys.has(scopedKey) ||
        (currentArrayIndex === -1 && definedTables.has(pathKey(fullPath)))
      ) {
        throw new Error(`duplicate key definition: ${scopedKey}`);
      }
      definedKeys.add(scopedKey);
      checkPath(fullPath);
      cursor.scanValue(
        fullPath,
        (subPath) => {
          const sk = `${currentArrayIndex}:${pathKey(subPath)}`;
          if (definedKeys.has(sk)) throw new Error(`duplicate inline key: ${sk}`);
          definedKeys.add(sk);
          checkPath(subPath);
        },
        0,
      );
      if (found) return true;
      cursor.skipSpaces();
      if (cursor.peek() === "#") cursor.skipCommentToEol();
      if (!cursor.eof() && cursor.peek() !== "\n" && cursor.peek() !== "\r") {
        throw new Error("trailing tokens after assignment");
      }
    }
    return found;
  } catch {
    // Structural TOML parse failure: fail closed so an explicit opt-out is never missed.
    return true;
  }
}

/**
 * Read a bounded regular file without blocking on FIFOs or exhausting memory.
 * Returns `{ status: "missing" }`, `{ status: "ok", text }`, or
 * `{ status: "unreadable" }` (caller must fail closed).
 */
export function readBoundedRegularFile(path, maxBytes = MAX_CONFIG_BYTES) {
  if (typeof path !== "string" || !isAbsolute(path)) return { status: "unreadable" };
  let fd;
  try {
    fd = openSync(path, fsConstants.O_RDONLY | (fsConstants.O_NONBLOCK ?? 0));
  } catch (error) {
    if (error?.code === "ENOENT" || error?.code === "ENOTDIR") {
      return { status: "missing" };
    }
    return { status: "unreadable" };
  }
  try {
    const stat = fstatSync(fd);
    if (!stat.isFile() || stat.size > maxBytes) {
      return { status: "unreadable" };
    }
    const chunks = [];
    let total = 0;
    const scratch = Buffer.allocUnsafe(8192);
    for (;;) {
      const n = readSync(fd, scratch, 0, scratch.length, null);
      if (n === 0) break;
      total += n;
      if (total > maxBytes) {
        return { status: "unreadable" };
      }
      chunks.push(Buffer.from(scratch.subarray(0, n)));
    }
    return { status: "ok", text: Buffer.concat(chunks, total).toString("utf8") };
  } catch {
    return { status: "unreadable" };
  } finally {
    try {
      closeSync(fd);
    } catch {}
  }
}

function fileHasOtelPolicy(path) {
  if (typeof path !== "string" || !isAbsolute(path)) return false;
  const read = readBoundedRegularFile(path, MAX_CONFIG_BYTES);
  if (read.status === "missing") return false;
  if (read.status !== "ok") return true;
  return tomlHasOtelPolicy(read.text);
}

function ancestorProjectConfigPaths(cwd) {
  if (typeof cwd !== "string" || !isAbsolute(cwd)) return [];
  const paths = [];
  let dir = resolve(cwd);
  for (let depth = 0; depth < 16; depth++) {
    paths.push(join(dir, ".codex", "config.toml"));
    if (existsSync(join(dir, ".git"))) break;
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return paths;
}

/**
 * Ask the exact pinned `codex` binary to evaluate effective config and
 * requirements across all layers (user, project, system, managed, cloud/MDM)
 * using the launch's `-c` overrides. Refuses token refresh so named accounts
 * can never be rotated during probing. Returns `null` on any error, malformed
 * reply, or timeout so callers fail closed.
 */
export function probeCodexOtelConfig(
  codexPath,
  cwd,
  env = process.env,
  timeoutMs = 5000,
  configuration = [],
) {
  if (!isAbsolute(codexPath ?? "")) return Promise.resolve(null);
  return new Promise((resolveProbe) => {
    let settled = false;
    let exited = false;
    let killTimer;
    let child;

    const terminateChild = () => {
      if (!child) return;
      try {
        child.stdin?.destroy();
      } catch {}
      try {
        child.stdout?.destroy();
      } catch {}
      if (exited) return;
      try {
        child.kill("SIGTERM");
      } catch {}
      if (!killTimer) {
        killTimer = setTimeout(() => {
          if (!exited) {
            try {
              child.kill("SIGKILL");
            } catch {}
          }
        }, 500);
        killTimer.unref?.();
      }
    };

    const finish = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      terminateChild();
      resolveProbe(value);
    };

    const childEnv = { ...env };
    delete childEnv[CODEX_OBSERVATION_ENV];
    delete childEnv.NODE_DEBUG;
    delete childEnv.NODE_DEBUG_NATIVE;
    // Match plugins/codex/collector: prevent proactive auth.json rotation on probe.
    childEnv.CODEX_REFRESH_TOKEN_URL_OVERRIDE = REFRESH_REFUSED_URL;

    const appServerArgs = [
      "app-server",
      "--stdio",
      ...(Array.isArray(configuration) ? configuration : []),
      "-c",
      "features.memories=false",
      "-c",
      "analytics.enabled=false",
    ];
    try {
      child = spawn(codexPath, appServerArgs, {
        cwd: typeof cwd === "string" && isAbsolute(cwd) ? cwd : undefined,
        env: childEnv,
        stdio: ["pipe", "pipe", "ignore"],
      });
    } catch {
      finish(null);
      return;
    }

    const timer = setTimeout(() => {
      if (!exited) {
        try {
          child.kill("SIGKILL");
        } catch {}
      }
      finish(null);
    }, timeoutMs);

    child.once("error", () => {
      exited = true;
      clearTimeout(killTimer);
      finish(null);
    });
    child.once("exit", () => {
      exited = true;
      clearTimeout(killTimer);
      finish(null);
    });
    child.stdin.on("error", () => finish(null));
    child.stdout.on("error", () => finish(null));

    let buffer = "";
    let initialized = false;
    let configRead;
    let reqRead;
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      if (settled) return;
      buffer += chunk;
      if (buffer.length > 1024 * 1024) {
        finish(null);
        return;
      }
      let newline;
      while ((newline = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, newline).trim();
        buffer = buffer.slice(newline + 1);
        if (!line) continue;
        let message;
        try {
          message = JSON.parse(line);
        } catch {
          finish(null);
          return;
        }
        if (!isPlainObject(message)) {
          finish(null);
          return;
        }
        if (message.id === 1) {
          if (initialized || message.error !== undefined || !isPlainObject(message.result)) {
            finish(null);
            return;
          }
          initialized = true;
          try {
            child.stdin.write(
              `${JSON.stringify({ method: "initialized", params: {} })}\n` +
                `${JSON.stringify({
                  id: 2,
                  method: "config/read",
                  params: { includeLayers: true, cwd },
                })}\n` +
                `${JSON.stringify({ id: 3, method: "configRequirements/read", params: {} })}\n`,
            );
          } catch {
            finish(null);
            return;
          }
          continue;
        }
        if (message.id === 2) {
          if (
            !initialized ||
            configRead !== undefined ||
            message.error !== undefined ||
            !isPlainObject(message.result)
          ) {
            finish(null);
            return;
          }
          const res = message.result;
          if (
            !isPlainObject(res.config) ||
            !isPlainObject(res.origins) ||
            !Array.isArray(res.layers) ||
            res.layers.some(
              (layer) =>
                !isPlainObject(layer) ||
                (layer.name !== undefined && !isPlainObject(layer.name)) ||
                (layer.config !== undefined && !isPlainObject(layer.config)),
            )
          ) {
            finish(null);
            return;
          }
          configRead = res;
        } else if (message.id === 3) {
          if (
            !initialized ||
            reqRead !== undefined ||
            message.error !== undefined ||
            !isPlainObject(message.result)
          ) {
            finish(null);
            return;
          }
          const res = message.result;
          if (
            !Object.hasOwn(res, "requirements") ||
            (res.requirements !== null && !isPlainObject(res.requirements))
          ) {
            finish(null);
            return;
          }
          reqRead = res;
        }
        if (configRead !== undefined && reqRead !== undefined) {
          finish({
            config: configRead.config,
            origins: configRead.origins,
            layers: configRead.layers,
            requirements: reqRead.requirements,
          });
          return;
        }
      }
    });
    try {
      child.stdin.write(
        `${JSON.stringify({
          id: 1,
          method: "initialize",
          params: { clientInfo: { name: "cowboy-codex-probe", version: "1" }, capabilities: {} },
        })}\n`,
      );
    } catch {
      finish(null);
    }
  });
}

function hasOtelFields(otel) {
  if (!isPlainObject(otel)) return false;
  return (
    otel.exporter != null ||
    otel.headers_helper != null ||
    otel.trace_exporter != null ||
    otel.metrics_exporter != null
  );
}

/**
 * Returns `true` when any existing configuration layer, CLI flag, environment
 * variable, split/remote placement, or unreadable/failed probe forbids
 * attaching Cowboy's observation exporter (fail closed).
 */
export async function hasExplicitCodexOtelPolicy({
  codexPath,
  configuration = [],
  cwd = process.cwd(),
  env = process.env,
  probeConfig = probeCodexOtelConfig,
}) {
  if (hasEnvDumpingNodeDebug(env)) {
    return true;
  }
  // Execution-bound sessions target a workspace that may reside on another
  // Machine while process.cwd() is a private runtime dir; decline observation
  // because the target project's .codex/config.toml cannot be locally verified.
  if (env.COWBOY_EXECUTION_DESCRIPTOR) {
    return true;
  }
  if (
    configuration.some(
      (value, index) =>
        index % 2 === 1 &&
        typeof value === "string" &&
        /^(?:otel|["']otel["'])(?:\.|=|\s*=)/.test(value.trim()),
    )
  ) {
    return true;
  }
  if (Object.keys(env).some((name) => name.startsWith("OTEL_") && env[name] !== "")) {
    return true;
  }
  const home = env.CODEX_HOME ?? (env.HOME ? join(env.HOME, ".codex") : "");
  const staticFiles = new Set([
    ...(home ? [join(home, "config.toml")] : []),
    "/etc/codex/config.toml",
    ...ancestorProjectConfigPaths(cwd),
  ]);
  for (const path of staticFiles) {
    if (fileHasOtelPolicy(path)) return true;
  }
  let probed;
  try {
    probed = await probeConfig(codexPath, cwd, env, 5000, configuration);
  } catch {
    return true;
  }
  if (
    !isPlainObject(probed) ||
    !isPlainObject(probed.config) ||
    !isPlainObject(probed.origins) ||
    !Array.isArray(probed.layers) ||
    probed.layers.some(
      (layer) =>
        !isPlainObject(layer) ||
        (layer.name !== undefined && !isPlainObject(layer.name)) ||
        (layer.config !== undefined && !isPlainObject(layer.config)),
    ) ||
    !Object.hasOwn(probed, "requirements") ||
    (probed.requirements !== null && !isPlainObject(probed.requirements))
  ) {
    return true;
  }
  if (hasOtelFields(probed.config.otel) || hasOtelFields(probed.requirements?.otel)) {
    return true;
  }
  if (
    probed.origins &&
    Object.keys(probed.origins).some((key) =>
      /^otel\.(?:exporter|headers_helper|trace_exporter|metrics_exporter)\b/.test(key),
    )
  ) {
    return true;
  }
  for (const layer of Array.isArray(probed.layers) ? probed.layers : []) {
    if (hasOtelFields(layer?.config?.otel)) return true;
    const dotCodex = layer?.name?.dotCodexFolder;
    if (typeof dotCodex === "string" && fileHasOtelPolicy(join(dotCodex, "config.toml"))) {
      return true;
    }
    const file = layer?.name?.file;
    if (typeof file === "string" && fileHasOtelPolicy(file)) {
      return true;
    }
  }
  return false;
}

/**
 * Build Codex `-c` overrides for Cowboy's loopback OTLP HTTP logs receiver.
 * Never places `header_value` in argv; Codex retrieves the header dynamically
 * from `helperPath` (`bin/codex-otel-headers`).
 */
export function codexObservationConfiguration({ binding, helperPath }) {
  if (!binding || !isAbsolute(helperPath ?? "")) {
    throw new Error("Codex request observation requires an absolute headers_helper path");
  }
  return [
    "-c",
    `otel.exporter={"otlp-http"={endpoint=${JSON.stringify(binding.logs_endpoint)},protocol="json",headers={}}}`,
    "-c",
    `otel.headers_helper=${JSON.stringify(helperPath)}`,
    "-c",
    "otel.log_user_prompt=false",
    "-c",
    "otel.log_agent_responses=false",
    "-c",
    "otel.log_sse_deltas=false",
  ];
}

/**
 * Decide whether an ordinary Codex launch should attach Cowboy's loopback OTLP
 * logs observation binding. Returns `{ args, keepEnv }`; callers MUST keep
 * `COWBOY_PRIVATE_CODEX_REQUEST_OBSERVATION` stripped whenever `keepEnv` is false.
 */
export async function resolveCodexObservation({
  codexPath,
  configuration = [],
  cwd = process.cwd(),
  env = process.env,
  rawBinding = env?.[CODEX_OBSERVATION_ENV],
  helperPath,
  probeConfig = probeCodexOtelConfig,
}) {
  if (hasEnvDumpingNodeDebug(env) || env?.COWBOY_EXECUTION_DESCRIPTOR) {
    return { args: [], keepEnv: false };
  }
  const binding = parseObservationBinding(rawBinding);
  if (
    !binding ||
    !isAbsolute(codexPath ?? "") ||
    !isAbsolute(helperPath ?? "") ||
    !existsSync(helperPath)
  ) {
    return { args: [], keepEnv: false };
  }
  const hasPolicy = await hasExplicitCodexOtelPolicy({
    codexPath,
    configuration,
    cwd,
    env,
    probeConfig,
  });
  if (hasPolicy) {
    return { args: [], keepEnv: false };
  }
  return {
    args: codexObservationConfiguration({ binding, helperPath }),
    keepEnv: true,
  };
}
