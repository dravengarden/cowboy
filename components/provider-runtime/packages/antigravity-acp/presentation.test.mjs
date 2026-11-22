import assert from "node:assert/strict";
import test from "node:test";

import {
  agyNotice,
  availableCommands,
  facadeTool,
  planUpdate,
  presentExtension,
  presentedTool,
  presentToolCall,
  resultContent,
} from "./presentation.mjs";

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
  assert.equal(run.title, "cargo test …");
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

test("agy's web built-ins show as a web search and URL reads", () => {
  assert.equal(presentedTool("search_web"), "search_web");
  assert.equal(facadeTool("search_web"), undefined);
  assert.equal(presentedTool("read_file"), undefined);
  // sess-1795107755038: agy titles a search `"<query>" on Web`.
  const search = presentToolCall(
    call(
      "search_web",
      { query: "Claude Code prompt cache TTL", toolSummary: "Search" },
      { kind: "search", title: '"Claude Code prompt cache TTL" on Web' },
    ),
    { root },
  );
  assert.equal(search.name, "WebSearch");
  assert.equal(search.kind, "search");
  assert.equal(search.title, "Claude Code prompt cache TTL");
  assert.deepEqual(search.rawInput, { query: "Claude Code prompt cache TTL" });
  assert.equal(search.locations, undefined);
  const fetch = presentToolCall(
    call("read_url_content", { Url: "https://agentclientprotocol.com/protocol/tool-calls" }),
    { root },
  );
  assert.equal(fetch.name, "WebFetch");
  assert.equal(fetch.kind, "fetch");
  assert.equal(fetch.title, "https://agentclientprotocol.com/protocol/tool-calls");
  const chunk = presentToolCall(call("view_content_chunk", { document_id: "d", position: 1 }), {
    root,
  });
  assert.equal(chunk.kind, "fetch");
  assert.equal(chunk.title, "Read fetched content");
  // A later update names the call only by ID.
  const update = presentToolCall(
    { sessionUpdate: "tool_call_update", toolCallId: "c1", status: "completed", title: "t" },
    { root, known: "search_web" },
  );
  assert.equal(update.kind, "search");
  assert.equal(update.title, "t");
});

const answer = (text, isError = false) => ({ content: [{ type: "text", text }], isError });

test("a facade call's answer becomes its content; a successful edit keeps its diff", () => {
  assert.deepEqual(resultContent("ReadFile", { path: "a.txt" }, answer("hello\n"), root), [
    { type: "content", content: { type: "text", text: "hello\n" } },
  ]);
  assert.deepEqual(
    resultContent("RunCommand", { command: "false" }, answer("Exit code: 1\n"), root),
    [{ type: "content", content: { type: "text", text: "Exit code: 1\n" } }],
  );
  const edit = { path: "a.txt", old_string: "a", new_string: "b", toolSummary: "Edit" };
  assert.deepEqual(resultContent("EditFile", edit, answer("Edited"), root), [
    { type: "diff", path: "/srv/work/repo/a.txt", oldText: "a", newText: "b" },
    { type: "content", content: { type: "text", text: "Edited" } },
  ]);
  // A refused edit shows why, not a diff that never happened.
  assert.deepEqual(
    resultContent("EditFile", edit, answer("old_string was not found", true), root),
    [{ type: "content", content: { type: "text", text: "old_string was not found" } }],
  );
  assert.deepEqual(resultContent("ReadFile", {}, { content: [] }, root), []);
  // An image ReadFile returned is shown with its line; an oversized one is not.
  const image = { type: "image", data: "AAAA", mimeType: "image/png" };
  const line = { type: "text", text: "Image /srv/work/repo/a.png (image/png, 3 bytes)" };
  assert.deepEqual(resultContent("ReadFile", { path: "a.png" }, { content: [line, image] }, root), [
    { type: "content", content: line },
    { type: "content", content: image },
  ]);
  const huge = { ...image, data: "A".repeat(2 * 1024 * 1024) };
  assert.equal(
    resultContent("ReadFile", { path: "a.png" }, { content: [line, huge] }, root).length,
    1,
  );
});

test("an extension's MCP result is shown block by block, bounded", () => {
  const image = { type: "image", data: "AAAA", mimeType: "image/png" };
  assert.deepEqual(
    resultContent("memory_search", {}, { content: [{ type: "text", text: "[]" }, image] }, root),
    [
      { type: "content", content: { type: "text", text: "[]" } },
      { type: "content", content: image },
    ],
  );
  const big = { type: "text", text: "x".repeat(700 * 1024) };
  const shown = resultContent("suger_read", {}, { content: [big, big] }, root);
  assert.equal(shown.length, 2);
  assert.match(shown[1].content.text, /not shown/);
});

test("an extension call shows as its MCP call without agy's narration", () => {
  const shown = presentExtension(
    call("mcp_cowboy_memory_search", { query: "cache", toolSummary: "Search memory" }),
    { server: "matrix", tool: "memory_search" },
  );
  assert.equal(shown.name, "mcp__matrix__memory_search");
  assert.equal(shown.kind, "other");
  assert.equal(shown.title, "matrix · memory_search");
  assert.deepEqual(shown.rawInput, { query: "cache" });
  const titled = presentExtension(
    { ...call("mcp_cowboy_cowboy_read", undefined), title: "Cowboy read" },
    { server: "cowboy", tool: "cowboy_read" },
  );
  assert.equal(titled.title, "Cowboy read");
  assert.equal(titled.rawInput, undefined);
});

test("UpdatePlan becomes an ACP plan", () => {
  assert.deepEqual(
    planUpdate({
      plan: [
        { step: " Read ", status: "completed" },
        { step: "Fix", status: "in_progress" },
      ],
    }),
    {
      sessionUpdate: "plan",
      entries: [
        { content: "Read", priority: "medium", status: "completed" },
        { content: "Fix", priority: "medium", status: "in_progress" },
      ],
    },
  );
  for (const bad of [undefined, {}, { plan: [] }, { plan: [{ step: "", status: "pending" }] }]) {
    assert.equal(planUpdate(bad), undefined);
  }
});

test("agy's own status lines become notices; the model's text does not", () => {
  const chunk = (text) => ({
    sessionUpdate: "agent_message_chunk",
    content: { type: "text", text },
  });
  assert.deepEqual(agyNotice(chunk("\n\u26a0 Model produced invalid output.\n")), {
    sessionUpdate: "notice",
    severity: "warning",
    title: "Antigravity: Model produced invalid output.",
  });
  assert.deepEqual(
    agyNotice(chunk("\n\u26a0 API error (attempt 1): UNAVAILABLE (code 503): No capacity\n")),
    {
      sessionUpdate: "notice",
      severity: "warning",
      title: "Antigravity API error (attempt 1)",
      description: "UNAVAILABLE (code 503): No capacity",
    },
  );
  assert.equal(
    agyNotice(
      chunk(
        "\n\u26a0 API error (attempt 2): UNAVAILABLE (code 503)\nCreated At: 2026-11-18T07:21:59Z\nCompleted At: 2026-11-18T07:21:59Z\n",
      ),
    )?.title,
    "Antigravity API error (attempt 2)",
  );
  for (const text of [
    "\u26a0 note",
    "\u26a0 note\nmore text",
    "Done. \u26a0 careful\n",
    "plain",
    "\n\u26a0 \n",
  ]) {
    assert.equal(agyNotice(chunk(text)), undefined, text);
  }
  assert.equal(
    agyNotice({ sessionUpdate: "agent_thought_chunk", content: chunk("\u26a0 x").content }),
    undefined,
  );
});

test("slash commands that need a denied tool are not offered", () => {
  const update = {
    sessionUpdate: "available_commands_update",
    availableCommands: [
      "goal",
      "compact",
      "schedule",
      "browser",
      "plan",
      "learn",
      "swarm",
      "run-workflow",
    ].map((name) => ({ name, description: name })),
  };
  assert.deepEqual(
    availableCommands(update).availableCommands.map((command) => command.name),
    ["goal", "compact", "plan"],
  );
  const kept = {
    sessionUpdate: "available_commands_update",
    availableCommands: [{ name: "goal" }],
  };
  assert.equal(availableCommands(kept), kept);
});

test("a multi-line command is titled by its first line", () => {
  const run = presentToolCall(call("mcp_cowboy_RunCommand", { command: "\nnode -e '\nx\n'" }), {
    root,
  });
  assert.equal(run.title, "node -e ' …");
});
