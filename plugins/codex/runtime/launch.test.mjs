import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  AGENT_CALLS_PROMPT,
  agentCallsConfiguration,
  agentCallsPrompt,
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

test("installed call profiles replace the built-in guidance for Codex", () => {
  const guidance =
    'To have another agent review or analyze work (for example when asked to call {{peer}}), use Cowboy instead of running `codex exec`, `codex review` or `claude -p` yourself. Run `cowboy {{peer_command}} --request-file - [--preset ID]` with this JSON on stdin: {"schema":1,"request_id":"<new unique id>","purpose":"review|design_review|analysis","instruction":"<self-contained task>","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}. It prints a JSON envelope with a call_id; then run `cowboy call wait <call_id>` until it is terminal and `cowboy call result <call_id>`. `cowboy call capabilities` reports whether calls are enabled and lists the preset ids (model and reasoning); omit --preset to use the session\'s choice. Reviews are optional and the user turns them on per session: when you have finished a substantive code change and are about to report, run `cowboy call capabilities`; if calls are enabled and {{peer}} is available, get a read-only review from {{peer}} first and fix or report its findings. If they are off, do not review and do not mention it. When the user explicitly asks for a call while calls are off, Cowboy asks them and the command waits for the answer; report a refusal instead of working around it.';
  const prompt = agentCallsPrompt({
    schema: 1,
    calls: { profiles: [{ guidance }] },
  });
  assert.match(prompt, /cowboy claude --request-file -/);
  assert.match(prompt, /review from Claude first/);
  assert.doesNotMatch(prompt, /\{\{/);
  assert.equal(agentCallsPrompt(null), AGENT_CALLS_PROMPT);
  assert.equal(
    agentCallsPrompt({ schema: 1, calls: null }),
    AGENT_CALLS_PROMPT,
  );
  const added = agentCallsConfiguration([], "", prompt);
  assert.equal(
    JSON.parse(added[1].slice("developer_instructions=".length)),
    prompt,
  );
});
