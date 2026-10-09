// The plugin's hook reproduction, checked case by case against native-local
// observations (tools/claude_hooks_native_baseline.json, written by
// tools/claude_hooks_native_probe.py). A candidate CLI whose hooks behave
// differently fails here once its probe is re-run.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  hookMatches,
  hookOutcome,
} from "../plugins/claude-code/runtime/context-mod.js";

const baseline = JSON.parse(
  readFileSync(
    new URL("./claude_hooks_native_baseline.json", import.meta.url),
    "utf8",
  ),
);
const cases = baseline.tool_hooks;

const run = (exitCode, stdout = "", stderr = "") => ({
  exitCode,
  stdout,
  stderr,
});
const json = (value) => run(0, JSON.stringify(value));
const specific = (event, fields) =>
  json({ hookSpecificOutput: { hookEventName: event, ...fields } });

// What each probe case's hook printed, and the command native names in its
// messages (the probe writes JSON outputs to <ROOT>/out0.json).
const shell = (text) => "cat > /dev/null; " + text;
const printed = "cat > /dev/null; cat <ROOT>/out0.json";
const HOOKS = {
  pre_exit2: [
    shell("echo PRE_BLOCK_STDERR >&2; exit 2"),
    run(2, "", "PRE_BLOCK_STDERR\n"),
  ],
  pre_exit1: [
    shell("echo PRE_FAIL_STDERR >&2; exit 1"),
    run(1, "", "PRE_FAIL_STDERR\n"),
  ],
  pre_deny: [
    printed,
    specific("PreToolUse", {
      permissionDecision: "deny",
      permissionDecisionReason: "PRE_DENY_REASON",
    }),
  ],
  pre_allow_update: [
    printed,
    specific("PreToolUse", {
      permissionDecision: "allow",
      updatedInput: { command: "printf updated >> ran.txt" },
    }),
  ],
  pre_ask: [
    printed,
    specific("PreToolUse", {
      permissionDecision: "ask",
      permissionDecisionReason: "PRE_ASK_REASON",
    }),
  ],
  pre_legacy_block: [
    printed,
    json({ decision: "block", reason: "LEGACY_BLOCK_REASON" }),
  ],
  pre_context: [
    printed,
    specific("PreToolUse", { additionalContext: "PRE_CONTEXT" }),
  ],
  pre_stop: [printed, json({ continue: false, stopReason: "PRE_STOP_REASON" })],
  pre_plain_stdout: [
    shell("echo PRE_PLAIN_STDOUT"),
    run(0, "PRE_PLAIN_STDOUT\n"),
  ],
  post_exit2: [
    shell("echo POST_FEEDBACK >&2; exit 2"),
    run(2, "", "POST_FEEDBACK\n"),
  ],
  post_block: [
    printed,
    json({ decision: "block", reason: "POST_BLOCK_REASON" }),
  ],
  post_context: [
    printed,
    specific("PostToolUse", { additionalContext: "POST_CONTEXT" }),
  ],
  post_stop: [
    printed,
    json({ continue: false, stopReason: "POST_STOP_REASON" }),
  ],
  post_exit1: [shell("echo POST_FAIL >&2; exit 1"), run(1, "", "POST_FAIL\n")],
  failure_exit2: [
    shell("echo FAIL_FEEDBACK >&2; exit 2"),
    run(2, "", "FAIL_FEEDBACK\n"),
  ],
  failure_stop: [
    printed,
    json({ continue: false, stopReason: "FAIL_STOP_REASON" }),
  ],
};

const resultOf = (observed) =>
  observed.model_tail.content.find((block) => block.type === "tool_result");

test("every reproduced hook outcome reads as native's", () => {
  for (const [name, [command, hookRun]] of Object.entries(HOOKS)) {
    const observed = cases[name];
    assert.ok(observed, name);
    const outcome = hookOutcome(observed.event, "Bash", command, hookRun);
    const visible = JSON.stringify(observed.model_tail);
    if (outcome.deny !== undefined) {
      const result = resultOf(observed);
      assert.equal(result.is_error, true, name);
      assert.ok(result.content.startsWith(outcome.deny), name);
      assert.equal(observed.ran, null, name);
    } else {
      assert.notEqual(observed.ran, null, name);
    }
    for (const note of outcome.context ?? []) {
      assert.ok(visible.includes(JSON.stringify(note).slice(1, -1)), name);
    }
    if (outcome.ask !== undefined) {
      assert.deepEqual(observed.asked, [outcome.ask], name);
    }
    if (outcome.allow && outcome.input) {
      assert.equal(observed.ran, "updated", name);
    }
    if (outcome.stop !== undefined && observed.event !== "PostToolUseFailure") {
      // The turn ends after the tool: native makes no follow-up request.
      assert.equal(observed.api_requests, 1, name);
    } else {
      assert.equal(observed.api_requests, 2, name);
    }
  }
});

test("a failure hook's continue:false does not end the turn natively", () => {
  // context-mod.js ignores it for this reason.
  assert.equal(cases.failure_stop.api_requests, 2);
  assert.equal(cases.pre_stop_then_fail.api_requests, 1);
});

test("matchers select hooks as native selects them", () => {
  for (const [name, observed] of Object.entries(cases)) {
    if (!name.startsWith("match_")) continue;
    assert.equal(
      hookMatches(observed.matcher, "Bash"),
      observed.ran === null,
      name,
    );
  }
});

test("a non-zero Bash exit is a tool error natively, and only failure hooks run", () => {
  const failed = resultOf(cases.bash_nonzero_post);
  assert.equal(failed.is_error, true);
  assert.match(failed.content, /^Exit code 3/);
  assert.equal(cases.bash_nonzero_post.hook_stdin, null);
  assert.equal(
    cases.bash_nonzero_failure.hook_stdin.hook_event_name,
    "PostToolUseFailure",
  );
  assert.ok(
    JSON.stringify(cases.pre_context_then_fail.model_tail).includes(
      "PreToolUse:Bash hook additional context: PRE_CONTEXT",
    ),
  );
});

test("PermissionRequest hooks race the host prompt as natively", () => {
  // An instant host decides before the hooks; a slow host lets the first
  // hook decision win, and a deny's message reaches the model verbatim.
  for (const name of ["permreq_allow", "permreq_deny", "permreq_exit2"]) {
    assert.equal(cases[name].ran, "original", name);
  }
  assert.equal(cases.permreq_fast_allow_slow_deny.ran, "original");
  assert.equal(cases.permreq_fast_deny_slow_allow.ran, null);
  assert.ok(
    resultOf(cases.permreq_fast_deny_slow_allow).content.startsWith(
      "DENY_0",
    ),
  );
  assert.equal(cases.permreq_none.hook_stdin.tool_use_id, undefined);
  assert.ok("permission_suggestions" in cases.permreq_none.hook_stdin);
});

test("a shell prefix gets shell-form hooks as one argument; exec form bypasses it", () => {
  const { plain, prefixed } = baseline.shell_prefix;
  assert.deepEqual(plain.hooks_ran, [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
  ]);
  // Facade (Mod-answered) Read calls never reach settings tool hooks.
  assert.equal(
    plain.hooks_ran.filter((name) => name === "PreToolUse").length,
    1,
  );
  const entries = prefixed.prefix_invocations;
  assert.ok(entries.length > 0);
  assert.ok(entries.every((entry) => entry.startsWith("argc=1\n")));
  assert.ok(entries.some((entry) => entry.includes("# BRACED")));
  assert.ok(!entries.some((entry) => entry.includes("EXEC_FORM")));
});

test("a subagent's hook input carries its identity and the main transcript", () => {
  for (const mode of ["foreground", "background"]) {
    const [pre, stop] = baseline.agent_hooks[mode].hooks;
    assert.equal(pre.hook_event_name, "PreToolUse", mode);
    assert.equal(pre.agent_type, "general-purpose", mode);
    assert.ok(pre.agent_id && pre.transcript_path, mode);
    assert.equal(stop.hook_event_name, "SubagentStop", mode);
    assert.ok(stop.agent_transcript_path.includes("/subagents/"), mode);
  }
});
