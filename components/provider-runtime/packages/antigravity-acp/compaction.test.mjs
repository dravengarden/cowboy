import assert from "node:assert/strict";
import test from "node:test";
import {
  Compaction,
  compactionStatus,
  isCompactionPrompt,
  isSlashPrompt,
  MAX_SESSIONS,
  supportsCompaction,
} from "./compaction.mjs";

const text = (value) => ({ type: "text", text: value });

test("only agy's /compact is a compaction prompt", () => {
  for (const prompt of [
    [text("/compact")],
    [text(" /Compact now ")],
    [text("/com"), text("pact")],
  ]) {
    assert.equal(isCompactionPrompt(prompt), true, JSON.stringify(prompt));
  }
  for (const prompt of [
    [],
    undefined,
    [text("/compaction")],
    [text("please /compact")],
    [text("/summarize")],
    [text("/compact"), { type: "image", data: "", mimeType: "image/png" }],
  ]) {
    assert.equal(isCompactionPrompt(prompt), false, JSON.stringify(prompt));
  }
  assert.equal(isSlashPrompt([text("  /plan")]), true);
  assert.equal(isSlashPrompt([text("plan /x")]), false);
  assert.equal(isSlashPrompt([]), false);
});

test("the typed update follows the worker's session.compaction capability", () => {
  assert.equal(supportsCompaction({ clientCapabilities: { session: { compaction: {} } } }), true);
  for (const value of [undefined, null, true, []]) {
    assert.equal(
      supportsCompaction({ clientCapabilities: { session: { compaction: value } } }),
      false,
    );
  }
  assert.deepEqual(compactionStatus({ result: { stopReason: "end_turn" } }), {
    status: "completed",
  });
  assert.deepEqual(compactionStatus({ result: { stopReason: "cancelled" } }), {
    status: "cancelled",
  });
  assert.deepEqual(compactionStatus({ error: { message: "x" } }), {
    status: "failed",
    error: "x",
  });
});

test("projections are bounded and a restatement is consumed once", () => {
  const compaction = new Compaction();
  for (let index = 0; index <= MAX_SESSIONS; index += 1) {
    compaction.opened(`s${index}`, `P${index}`);
  }
  assert.equal(compaction.projections.size, MAX_SESSIONS);
  assert.equal(compaction.projections.has("s0"), false);
  compaction.settled("s0", "completed");
  assert.equal(compaction.take("s0", [text("hi")]), undefined);
  compaction.settled("s1", "failed");
  assert.equal(compaction.take("s1", [text("hi")]), undefined);
  compaction.settled("s1", "completed");
  assert.equal(compaction.take("s1", [text("/goal")]), undefined);
  assert.match(compaction.take("s1", [text("hi")]), /\nP1\n<\/cowboy_context_restated>$/);
  assert.equal(compaction.take("s1", [text("hi")]), undefined);
});
