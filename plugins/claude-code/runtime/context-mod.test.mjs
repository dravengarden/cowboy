import assert from "node:assert/strict";
import test from "node:test";
import { targetCompactionResult, targetImageResult } from "./context-mod.js";

test("target image result retains media and target path without the native cache locator", () => {
  const image = { type: "image", source: { type: "base64", data: "fixture" } };
  const target = {
    type: "text",
    text: "Image source in workspace: /target/a.png",
  };
  const cached = {
    type: "text",
    text:
      "[Image: source: /runtime/projects/session/tool-results/mcp-cowboy_execution-blob-1.png]",
  };
  const event = {
    origin: { kind: "tool", tool: "ReadFile" },
    message: {
      role: "user",
      content: [
        {
          type: "tool_result",
          tool_use_id: "original",
          content: [image, cached, target],
        },
      ],
    },
  };
  const result = targetImageResult(event);
  assert.deepEqual(result.message.content[0], {
    type: "tool_result",
    tool_use_id: "original",
    content: [image, target],
  });
  assert.equal(event.message.content[0].content.length, 3);
  const other = { ...event, origin: { kind: "tool", tool: "another_server" } };
  assert.equal(targetImageResult(other), other);
  const textOnly = {
    ...event,
    message: {
      content: [
        { type: "tool_result", tool_use_id: "text", content: [cached] },
      ],
    },
  };
  assert.deepEqual(targetImageResult(textOnly), textOnly);
});

test("native compaction keeps its summary without suggesting a runtime transcript read", () => {
  const summary =
    "Kept target summary.\n\nIf you need specific details from before compaction (like exact code snippets, error messages, or content you generated), read the full transcript at: /runtime/session.jsonl\nContinue the conversation.";
  const event = {
    door: "compaction",
    origin: { kind: "engine" },
    message: {
      content: [
        { type: "text", text: summary },
      ],
    },
  };
  assert.equal(
    targetCompactionResult(event).message.content[0].text,
    "Kept target summary.\n\nContinue the conversation.",
  );
  const user = { ...event, door: "prompt" };
  assert.equal(targetCompactionResult(user), user);
});
