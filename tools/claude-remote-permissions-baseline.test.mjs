// The permission gate context-mod.js builds on native $.tool.check, checked
// against native-local observations (tools/claude_permissions_native_baseline.json,
// written by tools/claude_permissions_native_probe.py).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { checkDenial } from "../plugins/claude-code/runtime/context-mod.js";
import { dontAskDenial } from "../plugins/claude-code/runtime/launch.mjs";

const baseline = JSON.parse(
  readFileSync(
    new URL("./claude_permissions_native_baseline.json", import.meta.url),
    "utf8",
  ),
);
const tool = (call) => call.call.split(" ")[0];
// Native validates a file call (read first, unchanged since read) before
// asking; such a call fails without a prompt in any mode.
const invalid = (call) => /^<tool_use_error>/.test(call.result.text);

test("$.tool.check asks exactly where native prompts, in each prompting mode", () => {
  for (const mode of ["default", "acceptEdits", "bypassPermissions"]) {
    for (const call of baseline[mode].calls.filter((call) => !invalid(call))) {
      assert.equal(
        call.check === "ask",
        call.native_prompted,
        `${mode} ${call.call}`,
      );
    }
  }
});

test("a check's denial reads as native's tool result", () => {
  let denials = 0;
  for (const [mode, observed] of Object.entries(baseline)) {
    for (const call of observed.calls) {
      if (call.check !== "deny" || invalid(call)) {
        continue;
      }
      denials++;
      assert.equal(call.result.is_error, true, `${mode} ${call.call}`);
      assert.equal(
        checkDenial({ decision: "deny", reason: call.reason }),
        call.result.text,
        `${mode} ${call.call}`,
      );
    }
  }
  assert.ok(denials > 0);
});

test("dontAsk denies what would prompt, with native's message", () => {
  const asked = baseline.dontAsk.calls.filter((call) =>
    call.check === "ask" && !invalid(call)
  );
  assert.ok(asked.length > 0);
  for (const call of asked) {
    assert.equal(call.native_prompted, false, call.call);
    assert.equal(call.result.is_error, true, call.call);
    assert.equal(dontAskDenial(tool(call)), call.result.text, call.call);
  }
});

test("native validates a file call before asking", () => {
  const validated = baseline.default.calls.filter(invalid);
  assert.ok(validated.length > 0);
  for (const call of validated) {
    assert.equal(call.check, "ask", call.call);
    assert.equal(call.native_prompted, false, call.call);
  }
});
