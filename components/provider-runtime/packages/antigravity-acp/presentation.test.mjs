import assert from "node:assert/strict";
import test from "node:test";

import { facadeTool, presentToolCall } from "./presentation.mjs";

const root = "/srv/work/repo";
const call = (name, rawInput, extra = {}) => ({
  sessionUpdate: "tool_call",
  toolCallId: "c1",
  kind: "other",
  name,
  title: `Run ${name}`,
  status: "pending",
  rawInput,
  ...extra,
});

test("only the facade's tools are recognized", () => {
  assert.equal(facadeTool("mcp_cowboy_ReadFile"), "ReadFile");
  assert.equal(facadeTool("mcp_cowboy_constructor"), undefined);
  assert.equal(facadeTool("read_file"), undefined);
  assert.equal(facadeTool(undefined), undefined);
  const builtIn = call("run_command", { command: "id" });
  assert.equal(presentToolCall(builtIn, { root }), builtIn);
});

test("reads, lists and searches show their target path", () => {
  const read = presentToolCall(
    call("mcp_cowboy_ReadFile", { path: "AGENTS.md", toolSummary: "Read AGENTS.md" }),
    { root },
  );
  assert.equal(read.name, "Read");
  assert.equal(read.kind, "read");
  assert.equal(read.title, "Read AGENTS.md");
  assert.deepEqual(read.locations, [{ path: "/srv/work/repo/AGENTS.md" }]);
  assert.deepEqual(read.rawInput, { path: "AGENTS.md" });
  const list = presentToolCall(call("mcp_cowboy_ListDirectory", {}), { root });
  assert.equal(list.title, "List .");
  assert.deepEqual(list.locations, [{ path: root }]);
  const search = presentToolCall(
    call("mcp_cowboy_SearchFiles", { pattern: "fn main", path: "src" }),
    { root },
  );
  assert.equal(search.kind, "read");
  assert.equal(search.title, 'Search "fn main" in src');
});

test("writes and edits carry a diff ahead of their result", () => {
  const write = presentToolCall(
    call("mcp_cowboy_WriteFile", { path: "/abs/new.txt", content: "hello" }),
    { root },
  );
  assert.equal(write.kind, "edit");
  assert.equal(write.title, "Write /abs/new.txt");
  assert.deepEqual(write.content, [
    { type: "diff", path: "/abs/new.txt", oldText: null, newText: "hello" },
  ]);
  const result = { type: "content", content: { type: "text", text: "edited" } };
  const edit = presentToolCall(
    call(
      "mcp_cowboy_EditFile",
      { path: "a.rs", old_string: "x", new_string: "y" },
      { status: "completed", content: [result] },
    ),
    { root },
  );
  assert.equal(edit.title, "Edit a.rs");
  assert.deepEqual(edit.content, [
    { type: "diff", path: "/srv/work/repo/a.rs", oldText: "x", newText: "y" },
    result,
  ]);
});

test("commands show the command line and agy's summary as their description", () => {
  const run = presentToolCall(
    call("mcp_cowboy_RunCommand", {
      command: "cargo test\n--quiet",
      toolSummary: "Run the tests",
    }),
    { root },
  );
  assert.equal(run.kind, "execute");
  assert.equal(run.title, "cargo test");
  assert.deepEqual(run.rawInput, { command: "cargo test\n--quiet", description: "Run the tests" });
  assert.equal(run.locations, undefined);
  const more = presentToolCall(call("mcp_cowboy_RunCommand", { command_id: "p7" }), { root });
  assert.equal(more.title, "Continue p7");
  const stop = presentToolCall(call("mcp_cowboy_StopCommand", { command_id: "p7" }), { root });
  assert.equal(stop.title, "Stop p7");
  assert.equal(stop.name, "Stop");
});

test("an update without its name uses the recorded tool and keeps its title", () => {
  const update = {
    sessionUpdate: "tool_call_update",
    toolCallId: "c1",
    title: "Run mcp_cowboy_RunCommand",
    status: "completed",
    content: [{ type: "content", content: { type: "text", text: "ok" } }],
  };
  const shown = presentToolCall(update, { root, known: "RunCommand" });
  assert.equal(shown.kind, "execute");
  assert.equal(shown.name, "Run");
  assert.equal(Object.hasOwn(shown, "title"), false);
  assert.deepEqual(shown.content, update.content);
});
