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
  managedMcpOverrides,
  managedRuleViolations,
  splitConfigurationArguments,
} from "./launch.mjs";

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
  assert.throws(
    () => agentApiConfiguration({ environmentId: "target" }),
    /working directory/,
  );
});

test("Cowboy MCP avoids native table merges with existing server configuration", () => {
  const args = agentApiConfiguration({
    environmentId: "target",
    cwd: "/project",
    servers: ["cowboy", "cowboy_session", "cowboy_session_2", "other"],
  });
  assert.deepEqual(args.slice(0, 2), [
    "-c",
    "mcp_servers.cowboy.enabled=false",
  ]);
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
  assert.deepEqual(
    managedRuleViolations([rules, join(home, "missing")]),
    [],
  );
  writeFileSync(
    join(rules, "extra.rules"),
    'prefix_rule(pattern=["nix", "shell"], decision = "allow")\n',
  );
  assert.deepEqual(managedRuleViolations([rules]), [
    join(rules, "extra.rules"),
  ]);
});

test("managed profile disables every configured MCP server by exact name", () => {
  assert.deepEqual(managedMcpOverrides("[]"), []);
  assert.deepEqual(
    managedMcpOverrides(
      JSON.stringify([{ name: "chrome-devtools" }, { name: "matrix_2" }]),
    ),
    [
      "-c",
      "mcp_servers.chrome-devtools.enabled=false",
      "-c",
      "mcp_servers.matrix_2.enabled=false",
    ],
  );
  assert.throws(
    () => managedMcpOverrides('[{"name":"a.b"}]'),
    /cannot disable/,
  );
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
  assert.equal(
    JSON.parse(added[1].slice("developer_instructions=".length)),
    AGENT_CALLS_PROMPT,
  );
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
    agentCallsConfiguration(
      [],
      'model = "x"\ndeveloper_instructions = "mine"\n',
    ),
    [],
  );
  assert.deepEqual(
    agentCallsConfiguration(["-c", 'developer_instructions="cowboy"'], ""),
    [],
  );
  // A commented-out key is not a setting.
  assert.equal(
    agentCallsConfiguration([], '# developer_instructions = "old"\n').length,
    2,
  );
});

test("the Review Plugin renders Codex's guidance with Claude as its peer", () => {
  const { guidance } = JSON.parse(
    readFileSync(
      new URL("../../review/contract.json", import.meta.url),
      "utf8",
    ),
  );
  const prompt = agentCallsPrompt({
    schema: 1,
    calls: { profiles: [{ guidance }] },
  });
  assert.match(
    prompt,
    /Other delegation: MCP calls.start; fallback `cowboy claude --help`/,
  );
  assert.doesNotMatch(prompt, /\{\{|cowboy codex/);
  assert.equal(
    prompt,
    guidance.replaceAll("{{peer_command}}", "claude").replaceAll(
      "{{peer}}",
      "Claude",
    ),
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
  assert.equal(
    JSON.parse(added[1].slice("developer_instructions=".length)),
    prompt,
  );
});
