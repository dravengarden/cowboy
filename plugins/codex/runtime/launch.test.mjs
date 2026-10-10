import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  AGENT_CALLS_PROMPT,
  agentCallsConfiguration,
  managedMcpOverrides,
  managedRuleViolations,
  splitConfigurationArguments,
} from "./launch.mjs";

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
  assert.match(AGENT_CALLS_PROMPT, /cowboy claude --request-file -/);
  assert.doesNotMatch(AGENT_CALLS_PROMPT, /cowboy codex/);
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
