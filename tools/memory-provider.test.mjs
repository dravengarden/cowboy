import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import { codexObservation } from "../plugins/codex/runtime/memory.mjs";
import {
  claudeObservation,
  MATRIX_TOOLS,
  memoryToolNames,
} from "../plugins/claude-code/runtime/memory.mjs";
test("Claude permits the exact Matrix CodeAct tools", () => {
  assert.deepEqual(
    MATRIX_TOOLS,
    ["search", "get", "put", "forget", "read", "execute", "receipt"].map(
      (name) => "mcp__matrix__memory_" + name,
    ),
  );
});
test("Claude allows only the installed backend's enabled tools", () => {
  const config = {
    server: "matrix",
    tools: ["memory_search"],
    enableTools: true,
  };
  assert.deepEqual(memoryToolNames(config), ["mcp__matrix__memory_search"]);
  assert.deepEqual(memoryToolNames({ ...config, enableTools: false }), []);
  assert.deepEqual(memoryToolNames(null), []);
});
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

test("only memory.mjs imports the memory client package in a Provider runtime", () => {
  // The runtime builds rewrite that one file's package import to a bundled
  // copy; any other module importing the package fails once packaged.
  for (const plugin of ["claude-code", "codex"]) {
    const directory = new URL(`../plugins/${plugin}/runtime/`, import.meta.url);
    for (const name of readdirSync(directory)) {
      if (
        !/\.m?js$/.test(name) || name.includes(".test.") ||
        name === "memory.mjs"
      ) {
        continue;
      }
      const source = readFileSync(new URL(name, directory), "utf8");
      assert.doesNotMatch(
        source,
        /from "@cowboy\/memory-client"/,
        `${plugin}/runtime/${name} must import the memory client through memory.mjs`,
      );
    }
  }
});
