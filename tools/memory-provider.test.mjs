import assert from "node:assert/strict";
import test from "node:test";
import { codexObservation } from "../plugins/codex/runtime/memory.mjs";
import { claudeObservation } from "../plugins/claude-code/runtime/memory.mjs";
test("native capture accepts public completions and excludes thinking and images", () => {
  assert.equal(
    codexObservation({
      method: "item/completed",
      params: { item: { type: "reasoning", text: "private" } },
    }),
    null,
  );
  assert.deepEqual(
    codexObservation({
      method: "item/completed",
      params: { item: { type: "agentMessage", text: "Public answer" } },
    }),
    ["assistant", "Public answer"],
  );
  assert.deepEqual(
    claudeObservation({
      type: "assistant",
      message: {
        content: [
          { type: "thinking", thinking: "private" },
          { type: "image", data: "binary" },
          { type: "text", text: "Public answer" },
        ],
      },
    }),
    ["assistant", "Public answer"],
  );
  assert.equal(
    claudeObservation({
      type: "stream_event",
      event: { type: "thinking_delta" },
    }),
    null,
  );
});
