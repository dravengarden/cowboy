import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { resultStatus, resultText, toolContent, toolResult } from "./result";

test("ACP, MCP and Codex opaque results expose the same text", () => {
  const text = { type: "text", text: "ready" };
  for (
    const content of [
      "ready",
      [text],
      [{ type: "content", content: text }],
      { content: [text] },
      [{
        type: "tool_result",
        value: { result: { content: [text] }, error: null },
      }],
    ]
  ) assertEquals(resultText(content), "ready");
});

test("structured-only results and explicit empty scalar values are retained", () => {
  for (const value of [false, 0, null, [], {}, { sessions: [] }]) {
    const content = toolContent({ rawOutput: value, content: [] });
    assertEquals(resultText(content), JSON.stringify(value, null, 2));
    assertEquals(toolResult(content).recorded, true);
  }
  assertEquals(toolResult(undefined).recorded, false);
  assertEquals(toolResult([]).blocks, []);
  assertEquals(toolResult([]).recorded, true);
});

test("structured MCP text mirrors paint once without dropping extra text or images", () => {
  const value = { hits: [{ title: "test", snippet: "body" }] };
  const result = toolResult({
    content: [{ type: "text", text: JSON.stringify(value) }, {
      type: "text",
      text: "More available",
    }, { type: "image", mimeType: "image/png", data: "bytes" }],
    structuredContent: value,
  });
  assertEquals(result.blocks.map((b) => b.kind), ["text", "image", "data"]);
  assertEquals(result.blocks[2], { kind: "data", value });
});

test("MCP transport and tool errors override completed without guessing from prose", () => {
  for (
    const result of [
      { result: null, error: { message: "permission_denied" } },
      {
        result: {
          isError: true,
          content: [{ type: "text", text: "unavailable" }],
        },
        error: null,
      },
      { isError: true, content: [] },
    ]
  ) {
    const content = toolContent({ rawOutput: result });
    assertEquals(resultStatus("completed", content), "failed");
    assertEquals(resultStatus("in_progress", content), "in_progress");
  }
  assertEquals(
    resultStatus("completed", [{ type: "text", text: "No errors found" }]),
    "completed",
  );
});

test("resources and future result blocks remain inspectable", () => {
  const result = toolResult({
    content: [
      { type: "resource", resource: { uri: "file:///report", text: "hello" } },
      {
        type: "resource_link",
        uri: "https://example.test/report",
        name: "Report",
      },
      { type: "audio", mimeType: "audio/wav", data: "bytes" },
      { type: "future", payload: { meaningful: true } },
    ],
  });
  assertEquals(result.blocks.map((b) => b.kind), [
    "resource",
    "resource",
    "resource",
    "data",
  ]);
});

test("content retains priority over duplicate raw output and formatted files remain literal", () => {
  const content = [{ type: "text", text: "already here" }];
  assertEquals(toolContent({ content, rawOutput: "duplicate" }), content);
  assertEquals(toolContent({ rawOutput: { formatted_output: "file bytes" } }), [
    { type: "raw_output", text: "file bytes" },
  ]);
});
