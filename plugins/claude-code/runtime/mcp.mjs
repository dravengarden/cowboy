// The target's MCP servers for a bound session. Native Claude Code 2.1.287
// loads, in a local session (measured, SDK mode):
//
// - user servers: `mcpServers` of ~/.claude.json;
// - project servers: `.mcp.json` of each directory from the root down to the
//   working directory (nearer files override), without an approval prompt,
//   unless listed in the project's `disabledMcpjsonServers`;
// - local servers: `projects[<repository root or working directory>]
//   .mcpServers` of ~/.claude.json.
//
// A local server overrides a project server of the same name, which overrides
// a user server; `disabledMcpServers` turns a server off. `${VAR}` and
// `${VAR:-default}` expand from the environment Claude Code runs in, and a
// stdio server starts in the working directory with that environment.
//
// Natively these run beside the user's project, so here a stdio server runs on
// the target (mcp-proxy.mjs relays its stdio) and a remote server is reached
// from where the session runs, as WebFetch is.
import { posix } from "node:path";

const MAX_SERVERS = 64;

// Native's expansion of `${VAR}` and `${VAR:-default}`; a variable that is
// unset and has no default stays as written.
export function expandVariables(value, environment) {
  if (typeof value === "string") {
    return value.replace(
      /\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}/g,
      (whole, name, fallback) =>
        Object.hasOwn(environment, name)
          ? environment[name]
          : fallback ?? whole,
    );
  }
  if (Array.isArray(value)) {
    return value.map((item) => expandVariables(item, environment));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map((
        [key, item],
      ) => [key, expandVariables(item, environment)]),
    );
  }
  return value;
}

// The `projects` key native uses for the working directory.
export function projectKey(cwd, repositoryRoot) {
  return repositoryRoot ?? cwd;
}

// The directories whose `.mcp.json` native reads, root first.
export function projectConfigDirectories(cwd) {
  const directories = [];
  for (
    let directory = cwd;
    directory !== "/";
    directory = posix.dirname(directory)
  ) {
    directories.unshift(directory);
  }
  return directories;
}

function parse(text) {
  try {
    const value = JSON.parse(text);
    return value && typeof value === "object" && !Array.isArray(value)
      ? value
      : undefined;
  } catch {
    return undefined;
  }
}

function servers(value) {
  return value && typeof value === "object" && !Array.isArray(value)
    ? Object.entries(value).filter(([, server]) =>
      server && typeof server === "object" && !Array.isArray(server)
    )
    : [];
}

// A host name a local session would resolve to the user's own machine,
// which here is the target, not the machine this session runs on.
export function loopbackUrl(url) {
  let host;
  try {
    host = new URL(String(url)).hostname.toLowerCase().replace(/^\[|\]$/g, "");
  } catch {
    return false;
  }
  // A trailing dot names the same host; an IPv4-mapped address the same IP.
  host = host.replace(/\.$/, "").replace(/^::ffff:/, "");
  return host === "localhost" || host.endsWith(".localhost") ||
    host === "::1" || host === "0.0.0.0" || host === "::" ||
    /^127\.\d+\.\d+\.\d+$/.test(host) ||
    /^7f[0-9a-f]{2}:[0-9a-f]{1,4}$/.test(host);
}

// The servers native would load, in its precedence, with where each runs.
// `userConfig` is ~/.claude.json's text, `projectConfigs` the `.mcp.json`
// texts root first. Returns entries ready for native's --mcp-config and the
// servers this session cannot offer, with why.
export function targetMcpServers(
  { userConfig, projectConfigs, cwd, repositoryRoot, environment },
) {
  const user = parse(userConfig ?? "") ?? {};
  const local = user.projects?.[projectKey(cwd, repositoryRoot)] ?? {};
  const disabledProject = new Set(
    Array.isArray(local.disabledMcpjsonServers)
      ? local.disabledMcpjsonServers
      : [],
  );
  const disabled = new Set(
    Array.isArray(local.disabledMcpServers) ? local.disabledMcpServers : [],
  );
  const merged = new Map();
  for (const [name, server] of servers(user.mcpServers)) {
    merged.set(name, { scope: "user", server });
  }
  for (const text of projectConfigs) {
    for (const [name, server] of servers(parse(text)?.mcpServers)) {
      if (!disabledProject.has(name)) {
        merged.set(name, { scope: "project", server });
      }
    }
  }
  for (const [name, server] of servers(local.mcpServers)) {
    merged.set(name, { scope: "local", server });
  }
  const entries = [];
  const omitted = [];
  for (const [name, { scope, server }] of merged) {
    if (disabled.has(name)) continue;
    const omit = (reason) => omitted.push({ name, scope, reason });
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(name)) {
      omit("its name is not one this session can relay");
      continue;
    }
    if (entries.length >= MAX_SERVERS) {
      omit("the session's MCP server limit was reached");
      continue;
    }
    const expanded = expandVariables(server, environment);
    const type = expanded.type ?? (expanded.url ? "http" : "stdio");
    if (type === "stdio") {
      if (
        typeof expanded.command !== "string" || !expanded.command ||
        !(expanded.args === undefined ||
          (Array.isArray(expanded.args) &&
            expanded.args.every((item) => typeof item === "string"))) ||
        !(expanded.env === undefined ||
          (expanded.env && typeof expanded.env === "object" &&
            Object.values(expanded.env).every((item) =>
              typeof item === "string"
            )))
      ) {
        omit("its stdio configuration is invalid");
        continue;
      }
      entries.push({
        name,
        scope,
        placement: "target",
        argv: [expanded.command, ...(expanded.args ?? [])],
        env: expanded.env ?? {},
      });
      continue;
    }
    if (!["http", "sse"].includes(type) || typeof expanded.url !== "string") {
      omit(`its ${type} transport cannot be relayed here`);
      continue;
    }
    if (loopbackUrl(expanded.url)) {
      omit(
        "its URL names the target machine itself, which the machine running this session cannot reach",
      );
      continue;
    }
    // Native would expand what the target left unresolved from the
    // runtime's own environment (its credentials, its localhost).
    if (
      JSON.stringify([expanded.url, expanded.headers ?? null]).includes("${")
    ) {
      omit("it names environment variables the target does not set");
      continue;
    }
    if (expanded.headersHelper !== undefined) {
      omit("its headers helper would run on the machine running this session");
      continue;
    }
    entries.push({
      name,
      scope,
      placement: "runtime",
      config: {
        type,
        url: expanded.url,
        ...(expanded.headers && typeof expanded.headers === "object"
          ? { headers: expanded.headers }
          : {}),
        ...(expanded.oauth && typeof expanded.oauth === "object"
          ? { oauth: expanded.oauth }
          : {}),
      },
    });
  }
  return { entries, omitted };
}

// Native's --mcp-config servers: target stdio servers through the relay.
export function nativeMcpServers(entries, relay) {
  return Object.fromEntries(entries.map((entry) => [
    entry.name,
    entry.placement === "target"
      ? {
        type: "stdio",
        command: relay.command,
        args: [...relay.args, entry.name],
      }
      : entry.config,
  ]));
}

// The tool-name prefix native gives a server's tools.
export function mcpToolPrefix(name) {
  return "mcp__" + name.replace(/[^a-zA-Z0-9_-]/g, "_") + "__";
}
