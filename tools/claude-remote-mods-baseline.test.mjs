// The Mods behavior context-mod.js relies on, checked against native-local
// observations (tools/claude_mods_native_baseline.json, written by
// tools/claude_mods_native_probe.py), and the Mod source forms native refuses.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  recordHookBase,
  targetTaskNotification,
} from "../plugins/claude-code/runtime/context-mod.js";

const baseline = JSON.parse(
  readFileSync(
    new URL("./claude_mods_native_baseline.json", import.meta.url),
    "utf8",
  ),
);
const source = readFileSync(
  new URL("../plugins/claude-code/runtime/context-mod.js", import.meta.url),
  "utf8",
);
const classic = (scenario, name) =>
  baseline[scenario].classic.find((event) => event.name === name);

test("classic events carry the base hook input context-mod.js records", () => {
  for (const name of ["SessionStart", "UserPromptSubmit"]) {
    const { keys } = classic("background_idle", name);
    for (
      const key of [
        "session_id",
        "transcript_path",
        "prompt_id",
        "permission_mode",
      ]
    ) {
      assert.ok(keys.includes(key), `${name} ${key}`);
    }
  }
  // Effort first appears after a tool batch, as recordEffort assumes.
  assert.equal(classic("background_idle", "UserPromptSubmit").effort, null);
  assert.deepEqual(classic("background_idle", "PostToolBatch").effort, [
    "level",
  ]);
  const start = classic("background_idle", "SubagentStart");
  assert.equal(start.agent_type, "general-purpose");
  assert.equal(start.has_agent_id, true);
  assert.equal(typeof recordHookBase, "function");
});

test("turn.complete reports an agent's answer, and a stop as aborted", () => {
  const agent = (scenario) =>
    baseline[scenario].turns.filter((turn) => turn.agent);
  for (const key of ["agentId", "answer", "isAborted", "reason"]) {
    assert.ok(agent("background_idle")[0].keys.includes(key), key);
  }
  assert.deepEqual(
    agent("background_idle").map((
      { isAborted, reason },
    ) => [isAborted, reason]),
    [[false, "answer"]],
  );
  assert.deepEqual(
    agent("taskstop_held").map(({ isAborted, reason }) => [isAborted, reason]),
    [[true, "aborted"]],
  );
});

test("notifications arrive by the doors context-mod.js projects", () => {
  // Idle: appended as a prompt. Busy: a delivery append plus a queued_command
  // attachment the model reads. Both carry the same element.
  assert.deepEqual(baseline.background_idle.appends.map((item) => item.door), [
    "prompt",
  ]);
  assert.deepEqual(baseline.background_busy.appends.map((item) => item.door), [
    "delivery",
  ]);
  assert.match(baseline.background_busy.queued[0], /<task-notification>/);
  for (
    const door of [
      'door: "prompt"',
      'door: "delivery"',
      'type: "queued_command"',
    ]
  ) {
    assert.ok(source.includes(door), door);
  }
});

test("an agent notification is projected to its handle", () => {
  const content = JSON.parse(baseline.background_idle.appends[0].content);
  const text = content[0].text;
  const file = /<output-file>(.*?)<\/output-file>/.exec(text)[1];
  const id = /<task-id>([^<]*)<\/task-id>/.exec(text)[1];
  const projected = targetTaskNotification(
    {
      origin: { kind: "task-notification" },
      message: { role: "user", content },
    },
    new Map([[id, file]]),
  );
  const out = projected.message.content[0].text;
  assert.ok(out.includes(`<output-file>cowboy-agent://${id}</output-file>`));
  assert.ok(!out.includes(file));
});

test("a plugin's own background command is notified like the model's", () => {
  assert.equal(baseline.plugin_background.plugin_command_notified, true);
});

test("a pending Mod fetch holds back new turns but not a running turn's stop", () => {
  // mod-bridge.mjs keeps each observation short for the first of these.
  assert.equal(
    baseline.pending_fetch_parent.parent_turn_ran_during_pending_child_fetch,
    false,
  );
  assert.equal(
    baseline.taskstop_held.stop_processed_during_pending_child_fetch,
    true,
  );
  assert.equal(
    baseline.pending_fetch.child_progressed_during_pending_fetch,
    true,
  );
  // A stopped agent's held handler is not resumed, so the plugin ends its
  // target call itself (stoppedAgents) instead of waiting for next.signal.
  assert.equal(baseline.taskstop_held.held_handler_observed_after_stop, false);
  assert.ok(source.includes("stoppedAgents.add(event.agentId)"));
});

test("context-mod.js avoids the Mod source forms native disables", () => {
  assert.deepEqual(
    [
      baseline.start_api_access,
      baseline.receive_access,
      baseline.factory_handler,
      baseline.async_classic,
    ].map((item) => item.mod_active),
    [false, false, false, true],
  );
  // No handler built by a call: on("x", make()) or on("x", {...}, make()).
  for (
    const match of source.matchAll(
      /\bon\(\s*"[^"]+",\s*(?:\{[^}]*\},\s*)?([^\n]*)/g,
    )
  ) {
    assert.doesNotMatch(match[1], /^[A-Za-z_$][\w$]*\(/, match[0]);
  }
  // session.start touches neither $.session nor $.tool.
  const start = source.slice(source.indexOf('on("session.start"'));
  const body = start.slice(0, start.indexOf("\n  });"));
  assert.doesNotMatch(body, /\$\.(session|tool)\b/);
  assert.doesNotMatch(source, /\$\.session\.receive/);
});
