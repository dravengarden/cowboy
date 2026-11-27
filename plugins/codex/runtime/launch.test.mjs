import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  AGENT_CALLS_PROMPT,
  agentApiConfiguration,
  agentCallsConfiguration,
  agentCallsPrompt,
  API_KEY_AUTH_REQUEST,
  applyAwaitedMcpServers,
  applyDefaultAuthRequest,
  AWAIT_MCP_SERVERS_ENV,
  defaultAuthRequest,
  managedMcpOverrides,
  managedRuleViolations,
  splitConfigurationArguments,
} from "./launch.mjs";
import { spawnSync } from "node:child_process";
import { chmodSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { agentApiServerName, injectedMcpServerNames } from "./memory.mjs";
import { formatObservationHeaders } from "./otel-headers.mjs";
import {
  CODEX_OBSERVATION_ENV,
  CODEX_OBSERVATION_HEADER,
  codexObservationConfiguration,
  hasExplicitCodexOtelPolicy,
  parseObservationBinding,
  resolveCodexObservation,
  tomlHasOtelPolicy,
} from "./otel-observation.mjs";

test("Cowboy MCP follows native execution placement without copying credentials", () => {
  assert.deepEqual(agentApiConfiguration(), []);
  const remote = agentApiConfiguration({
    environmentId: "fixture-target",
    cwd: "/target/project",
    callContext: "/runtime/must-not-copy",
  });
  assert.match(remote[1], /environment_id="fixture-target"/);
  assert.match(remote[1], /cwd="\/target\/project"/);
  assert.match(remote[1], /env=\{COWBOY_CALL_CONTEXT=""\}/);
  assert.doesNotMatch(remote[1], /runtime\/must-not-copy|env_vars/);
  const local = agentApiConfiguration({ callContext: "/private/context" });
  assert.match(local[1], /env_vars=\["COWBOY_CALL_CONTEXT"\]/);
  assert.doesNotMatch(local[1], /private\/context|environment_id/);
  assert.match(local[1], /command="cowboy",args=\["mcp"\]/);
  assert.throws(() => agentApiConfiguration({ environmentId: "target" }), /working directory/);
});

test("Cowboy MCP avoids native table merges with existing server configuration", () => {
  const args = agentApiConfiguration({
    environmentId: "target",
    cwd: "/project",
    servers: ["cowboy", "cowboy_session", "cowboy_session_2", "other"],
  });
  assert.deepEqual(args.slice(0, 2), ["-c", "mcp_servers.cowboy.enabled=false"]);
  assert.match(args[3], /^mcp_servers.cowboy_session_3=\{/);
  assert.doesNotMatch(args.join(" "), /mcp_servers.other|\.url=|\.env=/);
  assert.deepEqual(agentApiConfiguration({ servers: ["cowboy"] }), []);
});

test("managed profile refuses sandbox-bypassing allow rules", () => {
  const home = mkdtempSync(join(tmpdir(), "codex-managed-"));
  const rules = join(home, "rules");
  mkdirSync(rules);
  writeFileSync(
    join(rules, "default.rules"),
    'prefix_rule(pattern=["git", "status"], decision="prompt")\n',
  );
  assert.deepEqual(managedRuleViolations([rules, join(home, "missing")]), []);
  writeFileSync(
    join(rules, "extra.rules"),
    'prefix_rule(pattern=["nix", "shell"], decision = "allow")\n',
  );
  assert.deepEqual(managedRuleViolations([rules]), [join(rules, "extra.rules")]);
});

test("managed profile disables every configured MCP server by exact name", () => {
  assert.deepEqual(managedMcpOverrides("[]"), []);
  assert.deepEqual(
    managedMcpOverrides(JSON.stringify([{ name: "chrome-devtools" }, { name: "matrix_2" }])),
    ["-c", "mcp_servers.chrome-devtools.enabled=false", "-c", "mcp_servers.matrix_2.enabled=false"],
  );
  assert.throws(() => managedMcpOverrides('[{"name":"a.b"}]'), /cannot disable/);
  assert.throws(() => managedMcpOverrides('{"servers":[]}'), /could not read/);
});

test("managed profile arguments keep validated configuration pairs", () => {
  const split = splitConfigurationArguments([
    "-c",
    "sandbox_mode=read-only",
    "-c",
    "features.hooks=false",
  ]);
  assert.deepEqual(split.configuration, [
    "-c",
    "sandbox_mode=read-only",
    "-c",
    "features.hooks=false",
  ]);
  assert.deepEqual(split.arguments, []);
});

test("ordinary sessions learn Cowboy calls without replacing user instructions", () => {
  const added = agentCallsConfiguration(["-c", "approval_policy=never"], "");
  assert.equal(added[0], "-c");
  assert.match(added[1], /^developer_instructions="/);
  // The value is a TOML basic string the native CLI parses back intact.
  assert.equal(JSON.parse(added[1].slice("developer_instructions=".length)), AGENT_CALLS_PROMPT);
  assert.match(AGENT_CALLS_PROMPT, /`cowboy review`/);
  assert.match(AGENT_CALLS_PROMPT, /`cowboy claude --help`/);
  assert.doesNotMatch(AGENT_CALLS_PROMPT, /cowboy codex/);
  // The agent decides, skipping is the default, and the always-on text is a
  // pointer rather than a manual.
  assert.match(AGENT_CALLS_PROMPT, /Skip it by default/);
  assert.ok(AGENT_CALLS_PROMPT.length < 900, AGENT_CALLS_PROMPT.length);
  assert.doesNotMatch(AGENT_CALLS_PROMPT, /capabilities|"schema"/);
  // The user's own developer instructions, or Cowboy's, are never replaced.
  assert.deepEqual(
    agentCallsConfiguration([], 'model = "x"\ndeveloper_instructions = "mine"\n'),
    [],
  );
  assert.deepEqual(agentCallsConfiguration(["-c", 'developer_instructions="cowboy"'], ""), []);
  // A commented-out key is not a setting.
  assert.equal(agentCallsConfiguration([], '# developer_instructions = "old"\n').length, 2);
});

test("the Review Plugin renders Codex's guidance with Claude as its peer", () => {
  const { guidance } = JSON.parse(
    readFileSync(new URL("../../review/contract.json", import.meta.url), "utf8"),
  );
  const prompt = agentCallsPrompt({
    schema: 1,
    calls: { profiles: [{ guidance }] },
  });
  assert.match(prompt, /Other delegation: MCP calls.start; fallback `cowboy claude --help`/);
  assert.doesNotMatch(prompt, /\{\{|cowboy codex/);
  assert.equal(
    prompt,
    guidance.replaceAll("{{peer_command}}", "claude").replaceAll("{{peer}}", "Claude"),
  );
  assert.match(prompt, /at most once per task/);
  assert.match(prompt, /cowboy review --ask/);
  assert.match(prompt, /Prefer Cowboy MCP/);
  assert.match(prompt, /review.start/);
  assert.match(prompt, /inspect calls.result/);
  assert.match(prompt, /review_selection/);
  // Review 1.0.6 guidance (notify-based CLI reviews) is 854 characters.
  assert.ok(prompt.length < 900);
  assert.equal(agentCallsPrompt(null), AGENT_CALLS_PROMPT);
  const added = agentCallsConfiguration([], "", prompt);
  assert.equal(JSON.parse(added[1].slice("developer_instructions=".length)), prompt);
});

test("only a projected API key selects the adapter's API-key login", () => {
  // A ChatGPT account, valid or expired, must reach ACP authRequired.
  assert.equal(defaultAuthRequest({}), undefined);
  assert.equal(defaultAuthRequest({ OPENAI_API_KEY: "  " }), undefined);
  assert.equal(defaultAuthRequest({ DEFAULT_AUTH_REQUEST: API_KEY_AUTH_REQUEST }), undefined);
  for (const name of ["OPENAI_API_KEY", "CODEX_API_KEY"]) {
    assert.equal(defaultAuthRequest({ [name]: "sk-fixture" }), API_KEY_AUTH_REQUEST);
  }
  assert.deepEqual(JSON.parse(API_KEY_AUTH_REQUEST), { methodId: "api-key" });
  // A request carrying its own key, or another method, is kept as declared.
  const inline = JSON.stringify({
    methodId: "api-key",
    _meta: { "api-key": { apiKey: "sk-inline" } },
  });
  assert.equal(defaultAuthRequest({ DEFAULT_AUTH_REQUEST: inline }), inline);
  const chatgpt = JSON.stringify({ methodId: "chat-gpt" });
  assert.equal(defaultAuthRequest({ DEFAULT_AUTH_REQUEST: chatgpt }), chatgpt);
  assert.throws(
    () => defaultAuthRequest({ DEFAULT_AUTH_REQUEST: "{" }),
    /Invalid default Codex authentication request/,
  );

  const chatgptAccount = { DEFAULT_AUTH_REQUEST: API_KEY_AUTH_REQUEST };
  applyDefaultAuthRequest(chatgptAccount);
  assert.equal("DEFAULT_AUTH_REQUEST" in chatgptAccount, false);
  const keyAccount = { OPENAI_API_KEY: "sk-fixture" };
  applyDefaultAuthRequest(keyAccount);
  assert.equal(keyAccount.DEFAULT_AUTH_REQUEST, API_KEY_AUTH_REQUEST);
});

test("the package no longer declares an unconditional API-key login", () => {
  const manifest = JSON.parse(readFileSync(new URL("../provider.json", import.meta.url), "utf8"));
  assert.equal("DEFAULT_AUTH_REQUEST" in manifest.runtime.environment, false);
  assert.deepEqual(manifest.authentication.environment_projection, {
    OPENAI_API_KEY: "api_key",
  });
});

test("the adapter awaits exactly the MCP servers the bridge injects", () => {
  assert.equal(agentApiServerName([]), "cowboy");
  assert.equal(agentApiServerName(["cowboy"]), "cowboy_session");
  assert.equal(
    agentApiServerName(["cowboy", "cowboy_session", "cowboy_session_2"]),
    "cowboy_session_3",
  );
  const matrix = {
    server: "matrix",
    tools: ["memory_search"],
    enableTools: true,
  };
  const listed = [];
  const listServers = () => {
    listed.push(true);
    return ["cowboy", "chrome-devtools"];
  };
  assert.deepEqual(injectedMcpServerNames({ cowboy: true, memory: matrix, listServers }), [
    "cowboy_session",
    "matrix",
  ]);
  // The injected Cowboy name avoids the Matrix name exactly as the bridge does.
  assert.deepEqual(
    injectedMcpServerNames({
      cowboy: true,
      memory: { ...matrix, server: "cowboy_session" },
      listServers,
    }),
    ["cowboy_session_2", "cowboy_session"],
  );
  // Matrix without enabled tools injects no MCP server; recall still works.
  assert.deepEqual(
    injectedMcpServerNames({
      cowboy: true,
      memory: { ...matrix, enableTools: false },
      listServers: () => [],
    }),
    ["cowboy"],
  );
  listed.length = 0;
  assert.deepEqual(injectedMcpServerNames({ cowboy: false, memory: matrix, listServers }), [
    "matrix",
  ]);
  assert.deepEqual(injectedMcpServerNames({ cowboy: false, memory: null, listServers }), []);
  assert.equal(listed.length, 0, "only Cowboy MCP naming lists native servers");
  // The listed user servers are never awaited.
  assert.doesNotMatch(
    JSON.stringify(injectedMcpServerNames({ cowboy: true, memory: null, listServers })),
    /chrome-devtools/,
  );

  const environment = { [AWAIT_MCP_SERVERS_ENV]: '["stale"]' };
  applyAwaitedMcpServers([], environment);
  assert.equal(AWAIT_MCP_SERVERS_ENV in environment, false);
  applyAwaitedMcpServers(["cowboy", "matrix"], environment);
  assert.deepEqual(JSON.parse(environment[AWAIT_MCP_SERVERS_ENV]), ["cowboy", "matrix"]);
});

test("Cowboy core MCP uses the verified CLI alongside binding servers", () => {
  const args = agentApiConfiguration({
    environmentId: "target",
    cwd: "/project",
    command: "/nix/store/cli/bin/cowboy",
  });
  assert.match(args[1], /command="\/nix\/store\/cli\/bin\/cowboy"/);
});

test("codex manifest opts into signed request observation binding", () => {
  const manifest = JSON.parse(readFileSync(new URL("../provider.json", import.meta.url), "utf8"));
  assert.equal(manifest.runtime.environment[CODEX_OBSERVATION_ENV], "codex-otlp-http-v1");
});

test("parseObservationBinding accepts only valid loopback v1 bindings", () => {
  const valid = JSON.stringify({
    schema: 1,
    logs_endpoint: "http://127.0.0.1:4318/v1/logs",
    header_name: CODEX_OBSERVATION_HEADER,
    header_value: "secret-token-123",
  });
  const parsed = parseObservationBinding(valid);
  assert.deepEqual(parsed, {
    schema: 1,
    logs_endpoint: "http://127.0.0.1:4318/v1/logs",
    header_name: CODEX_OBSERVATION_HEADER,
    header_value: "secret-token-123",
  });
  for (const bad of [
    "",
    "{",
    JSON.stringify({
      schema: 2,
      logs_endpoint: "http://127.0.0.1:4318/v1/logs",
      header_name: CODEX_OBSERVATION_HEADER,
      header_value: "x",
    }),
    JSON.stringify({
      schema: 1,
      logs_endpoint: "https://example.com/v1/logs",
      header_name: CODEX_OBSERVATION_HEADER,
      header_value: "x",
    }),
    JSON.stringify({
      schema: 1,
      logs_endpoint: "http://127.0.0.1:4318/v1/traces",
      header_name: CODEX_OBSERVATION_HEADER,
      header_value: "x",
    }),
    JSON.stringify({
      schema: 1,
      logs_endpoint: "http://127.0.0.1:4318/v1/logs?q=1",
      header_name: CODEX_OBSERVATION_HEADER,
      header_value: "x",
    }),
    JSON.stringify({
      schema: 1,
      logs_endpoint: "http://127.0.0.1:4318/v1/logs",
      header_name: "authorization",
      header_value: "x",
    }),
    JSON.stringify({
      schema: 1,
      logs_endpoint: "http://127.0.0.1:4318/v1/logs",
      header_name: CODEX_OBSERVATION_HEADER,
      header_value: "bad\nvalue",
    }),
  ]) {
    assert.equal(parseObservationBinding(bad), null, bad);
  }
});

test("codex observation respects explicit user/project/managed OTel policies and fails closed", async () => {
  const root = mkdtempSync(join(tmpdir(), "codex-otel-test-"));
  const home = join(root, "home");
  const cleanProject = join(root, "clean-proj");
  const policyProject = join(root, "policy-proj");
  mkdirSync(home, { recursive: true });
  mkdirSync(join(cleanProject, ".git"), { recursive: true });
  mkdirSync(join(policyProject, ".git"), { recursive: true });
  mkdirSync(join(policyProject, ".codex"), { recursive: true });
  writeFileSync(join(policyProject, ".codex", "config.toml"), '[otel]\nexporter = "none"\n');
  const helper = join(root, "codex-otel-headers");
  writeFileSync(helper, "#!/bin/sh\nexit 0\n");
  chmodSync(helper, 0o755);

  const bindingRaw = JSON.stringify({
    schema: 1,
    logs_endpoint: "http://127.0.0.1:51234/v1/logs",
    header_name: CODEX_OBSERVATION_HEADER,
    header_value: "super-secret-value",
  });
  const baseEnv = {
    HOME: root,
    CODEX_HOME: home,
    [CODEX_OBSERVATION_ENV]: bindingRaw,
  };
  const emptyProbe = async () => ({ config: {}, origins: {}, layers: [], requirements: {} });

  // Clean environment attaches exporter via headers_helper with zero secrets in argv.
  const attached = await resolveCodexObservation({
    codexPath: "/exact/codex",
    configuration: ["-c", "approval_policy=never"],
    cwd: cleanProject,
    env: baseEnv,
    helperPath: helper,
    probeConfig: emptyProbe,
  });
  assert.equal(attached.keepEnv, true);
  assert.ok(attached.args.length >= 6);
  assert.doesNotMatch(attached.args.join(" "), /super-secret-value/);
  assert.ok(attached.args.includes("otel.log_user_prompt=false"));
  assert.ok(attached.args.includes("otel.log_agent_responses=false"));
  assert.ok(attached.args.includes("otel.log_sse_deltas=false"));
  // Every -c pair matches codex-acp's strict configuration regex.
  const validated = splitConfigurationArguments(attached.args);
  assert.deepEqual(validated.arguments, []);

  // Project `.codex/config.toml` declaring `[otel] exporter = "none"` declines.
  const declinedProject = await resolveCodexObservation({
    codexPath: "/exact/codex",
    configuration: [],
    cwd: policyProject,
    env: baseEnv,
    helperPath: helper,
    probeConfig: emptyProbe,
  });
  assert.deepEqual(declinedProject, { args: [], keepEnv: false });

  // Effective managed requirements or user config exporter declines.
  assert.equal(
    await hasExplicitCodexOtelPolicy({
      codexPath: "/exact/codex",
      cwd: cleanProject,
      env: baseEnv,
      probeConfig: async () => ({
        config: { otel: { exporter: "none" } },
        origins: {},
        layers: [],
        requirements: {},
      }),
    }),
    true,
  );
  assert.equal(
    await hasExplicitCodexOtelPolicy({
      codexPath: "/exact/codex",
      cwd: cleanProject,
      env: baseEnv,
      probeConfig: async () => ({
        config: {},
        origins: {},
        layers: [],
        requirements: { otel: { headers_helper: "/opt/corp/helper" } },
      }),
    }),
    true,
  );
  // Probe failure fails closed.
  assert.equal(
    await hasExplicitCodexOtelPolicy({
      codexPath: "/exact/codex",
      cwd: cleanProject,
      env: baseEnv,
      probeConfig: async () => null,
    }),
    true,
  );
  // Explicit `-c otel.*` or `OTEL_*` env declines.
  assert.equal(
    await hasExplicitCodexOtelPolicy({
      codexPath: "/exact/codex",
      configuration: ["-c", 'otel.exporter="none"'],
      cwd: cleanProject,
      env: baseEnv,
      probeConfig: emptyProbe,
    }),
    true,
  );
  assert.equal(
    await hasExplicitCodexOtelPolicy({
      codexPath: "/exact/codex",
      cwd: cleanProject,
      env: { ...baseEnv, OTEL_EXPORTER_OTLP_ENDPOINT: "http://127.0.0.1:4318" },
      probeConfig: emptyProbe,
    }),
    true,
  );
  // Commented-out TOML does not count as an OTel policy.
  assert.equal(tomlHasOtelPolicy('# [otel]\n# exporter = "none"\nmodel = "gpt-6.2-sol"\n'), false);
  assert.equal(tomlHasOtelPolicy('[otel]\nexporter = "none"\n'), true);

  // Dynamic headers helper emits exact JSON header map from env without disk/argv secrets.
  assert.deepEqual(JSON.parse(formatObservationHeaders(baseEnv)), {
    [CODEX_OBSERVATION_HEADER]: "super-secret-value",
  });
  const helperScript = fileURLToPath(new URL("./otel-headers.mjs", import.meta.url));
  const res = spawnSync(process.execPath, [helperScript], {
    env: { PATH: process.env.PATH, [CODEX_OBSERVATION_ENV]: bindingRaw },
    encoding: "utf8",
  });
  assert.equal(res.status, 0);
  assert.deepEqual(JSON.parse(res.stdout), {
    [CODEX_OBSERVATION_HEADER]: "super-secret-value",
  });
  assert.throws(
    () =>
      codexObservationConfiguration({
        binding: parseObservationBinding(bindingRaw),
        helperPath: "relative",
      }),
    /absolute/,
  );
});
