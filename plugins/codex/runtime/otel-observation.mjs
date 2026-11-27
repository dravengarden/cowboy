// Signed Codex Level 2 per-request observation binding (`codex-otlp-http-v1`).
// Consumes the worker's private loopback OTLP HTTP logs binding ONLY when no
// user, project, system, managed, or CLI configuration already declares an
// OTel exporter or headers_helper policy. Fails closed on any probe error,
// and passes the auth header exclusively through `otel.headers_helper` (never
// in `-c` argv or on disk).
import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";

export const CODEX_OBSERVATION_ENV = "COWBOY_PRIVATE_CODEX_REQUEST_OBSERVATION";
export const CODEX_OBSERVATION_HEADER = "x-cowboy-request-observation";

/** Strictly parse and validate the worker's private observation descriptor. */
export function parseObservationBinding(raw) {
  if (typeof raw !== "string" || raw.length === 0 || raw.length > 4096) return null;
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
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

/** Whether raw TOML text declares any `[otel]` table or `otel.exporter`/`headers_helper` key. */
export function tomlHasOtelPolicy(text) {
  if (typeof text !== "string" || !text) return false;
  let inOtelTable = false;
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.replace(/#.*$/, "").trim();
    if (!line) continue;
    if (line.startsWith("[")) {
      inOtelTable = /^\[\s*otel(?:\.|\s*\])/.test(line);
      if (/^\[\s*otel\.(?:exporter|trace_exporter|metrics_exporter)\b/.test(line)) {
        return true;
      }
      continue;
    }
    if (
      inOtelTable &&
      /^(?:exporter|headers_helper|trace_exporter|metrics_exporter)\s*=/.test(line)
    ) {
      return true;
    }
    if (/^otel\.(?:exporter|headers_helper|trace_exporter|metrics_exporter)\b/.test(line)) {
      return true;
    }
  }
  return false;
}

function fileHasOtelPolicy(path) {
  if (typeof path !== "string" || !isAbsolute(path)) return false;
  try {
    return tomlHasOtelPolicy(readFileSync(path, "utf8"));
  } catch (error) {
    if (error?.code === "ENOENT" || error?.code === "ENOTDIR") return false;
    // Unreadable existing policy file: fail closed.
    return true;
  }
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
 * requirements across all layers (user, project, system, managed, cloud/MDM).
 * Returns `null` on any error or timeout so callers fail closed.
 */
export function probeCodexOtelConfig(codexPath, cwd, env = process.env, timeoutMs = 5000) {
  if (!isAbsolute(codexPath ?? "")) return Promise.resolve(null);
  return new Promise((resolveProbe) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolveProbe(value);
    };
    const childEnv = { ...env };
    delete childEnv[CODEX_OBSERVATION_ENV];
    let child;
    try {
      child = spawn(codexPath, ["app-server", "--stdio"], {
        cwd: typeof cwd === "string" && isAbsolute(cwd) ? cwd : undefined,
        env: childEnv,
        stdio: ["pipe", "pipe", "ignore"],
      });
    } catch {
      finish(null);
      return;
    }
    const timer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {}
      finish(null);
    }, timeoutMs);
    let buffer = "";
    let configRead;
    let reqRead;
    child.once("error", () => finish(null));
    child.once("exit", () => finish(null));
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      buffer += chunk;
      if (buffer.length > 2 * 1024 * 1024) {
        try {
          child.kill("SIGKILL");
        } catch {}
        finish(null);
        return;
      }
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";
      for (const line of lines) {
        if (!line.trim()) continue;
        let message;
        try {
          message = JSON.parse(line);
        } catch {
          continue;
        }
        if (message.id === 1) {
          if (message.error) {
            try {
              child.kill("SIGTERM");
            } catch {}
            finish(null);
            return;
          }
          try {
            child.stdin.write(`${JSON.stringify({ method: "initialized", params: {} })}\n`);
            child.stdin.write(
              `${JSON.stringify({
                id: 2,
                method: "config/read",
                params: { cwd: isAbsolute(cwd ?? "") ? cwd : null, includeLayers: true },
              })}\n`,
            );
            child.stdin.write(
              `${JSON.stringify({ id: 3, method: "configRequirements/read", params: {} })}\n`,
            );
          } catch {
            finish(null);
            return;
          }
        } else if (message.id === 2) {
          if (message.error) {
            try {
              child.kill("SIGTERM");
            } catch {}
            finish(null);
            return;
          }
          configRead = message.result ?? null;
        } else if (message.id === 3) {
          if (message.error) {
            try {
              child.kill("SIGTERM");
            } catch {}
            finish(null);
            return;
          }
          reqRead = message.result ?? null;
        }
        if (configRead !== undefined && reqRead !== undefined) {
          try {
            child.kill("SIGTERM");
          } catch {}
          finish({
            config: configRead?.config ?? null,
            origins: configRead?.origins ?? null,
            layers: Array.isArray(configRead?.layers) ? configRead.layers : [],
            requirements: reqRead?.requirements ?? null,
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
      try {
        child.kill("SIGKILL");
      } catch {}
      finish(null);
    }
  });
}

function hasOtelFields(otel) {
  if (!otel || typeof otel !== "object") return false;
  return (
    otel.exporter != null ||
    otel.headers_helper != null ||
    otel.trace_exporter != null ||
    otel.metrics_exporter != null
  );
}

/**
 * Returns `true` when any existing configuration layer, CLI flag, environment
 * variable, or unreadable/failed probe forbids attaching Cowboy's observation
 * exporter (fail closed).
 */
export async function hasExplicitCodexOtelPolicy({
  codexPath,
  configuration = [],
  cwd = process.cwd(),
  env = process.env,
  probeConfig = probeCodexOtelConfig,
}) {
  if (
    configuration.some(
      (value, index) => index % 2 === 1 && typeof value === "string" && /^otel(?:\.|=)/.test(value),
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
    probed = await probeConfig(codexPath, cwd, env);
  } catch {
    return true;
  }
  if (!probed || typeof probed !== "object") return true;
  if (hasOtelFields(probed.config?.otel) || hasOtelFields(probed.requirements?.otel)) {
    return true;
  }
  if (
    probed.origins &&
    typeof probed.origins === "object" &&
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
 * logs observation binding. Returns `{ args, keepEnv }`; callers MUST delete
 * `COWBOY_PRIVATE_CODEX_REQUEST_OBSERVATION` whenever `keepEnv` is false.
 */
export async function resolveCodexObservation({
  codexPath,
  configuration = [],
  cwd = process.cwd(),
  env = process.env,
  helperPath,
  probeConfig = probeCodexOtelConfig,
}) {
  const binding = parseObservationBinding(env[CODEX_OBSERVATION_ENV]);
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
