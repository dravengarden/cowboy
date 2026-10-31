import assert from "node:assert/strict";
import test from "node:test";
import {
  COWBOY_PATCH,
  dshHome,
  fileText,
  planOf,
  presentMessage,
  requestCwd,
} from "./launch.mjs";

const cwd = "/work/session";

function call(id, name, rawInput) {
  return {
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "s",
      update: {
        sessionUpdate: "tool_call",
        toolCallId: id,
        title: name,
        kind: "other",
        status: "in_progress",
        rawInput,
      },
    },
  };
}

function result(id, text, status = "completed") {
  return {
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "s",
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId: id,
        status,
        content: [{ type: "content", content: { type: "text", text } }],
      },
    },
  };
}

function run(name, rawInput, text, status) {
  const calls = new Map();
  const [started, ...extra] = presentMessage(
    call("c", name, rawInput),
    calls,
    cwd,
  );
  const [finished] = presentMessage(result("c", text, status), calls, cwd);
  return {
    started: started.params.update,
    finished: finished.params.update,
    extra,
    calls,
  };
}

test("bash becomes a described shell command with untouched output", () => {
  const { started, finished } = run(
    "bash",
    { command: "exit 3", description: "Intentionally fail" },
    "(no output)\n[exit code: 3]",
  );
  assert.equal(started.kind, "execute");
  assert.equal(started.title, "Intentionally fail");
  assert.deepEqual(started.rawInput, {
    command: "exit 3",
    description: "Intentionally fail",
  });
  assert.deepEqual(started._meta, { deepseekHarness: { toolName: "bash" } });
  assert.equal(finished.content[0].content.text, "(no output)\n[exit code: 3]");
  assert.equal(finished._meta.deepseekHarness.toolName, "bash");
});

test("read reports its location and strips dsh line framing", () => {
  const { started, finished } = run(
    "read",
    { file_path: "AGENTS.md", offset: 3, limit: 2 },
    "<path>/work/session/AGENTS.md</path>\n<type>file</type>\n<content>\n3: # Title\n4: \n</content>",
  );
  assert.equal(started.kind, "read");
  assert.equal(started.title, "Read AGENTS.md");
  assert.deepEqual(started.locations, [{
    path: "/work/session/AGENTS.md",
    line: 3,
  }]);
  assert.equal(finished.content[0].content.text, "# Title\n\n");
});

test("live read results lose line numbers and the end-of-file note only", () => {
  assert.equal(
    fileText("1: alpha\n2: \n3: gamma\n\n(End of file - total 3 lines)"),
    "alpha\n\ngamma\n",
  );
  assert.equal(
    fileText(
      "1: a\n2: b\n\n(Showing lines 1-2 of 9. Use offset=3 to continue.)",
    ),
    "a\nb\n\n(Showing lines 1-2 of 9. Use offset=3 to continue.)",
  );
  assert.equal(fileText("1: a\nnot numbered"), "1: a\nnot numbered");
});

test("write and edit render the diff they applied", () => {
  const write = run(
    "write",
    { file_path: "probe/notes.md", content: "one\ntwo\n" },
    "<path>/work/session/probe/notes.md</path>\n<type>file</type>\n<content>\nCreated file\n</content>",
  );
  assert.equal(write.started.kind, "edit");
  assert.equal(write.started.title, "Write probe/notes.md");
  assert.deepEqual(write.finished.content, [
    {
      type: "diff",
      path: "/work/session/probe/notes.md",
      oldText: null,
      newText: "one\ntwo\n",
    },
  ]);
  const edit = run(
    "edit",
    {
      file_path: "/work/session/probe/notes.md",
      old_string: "two",
      new_string: "2",
    },
    "The file has been updated successfully.",
  );
  assert.equal(edit.started.title, "Edit probe/notes.md");
  assert.deepEqual(edit.finished.content, [
    {
      type: "diff",
      path: "/work/session/probe/notes.md",
      oldText: "two",
      newText: "2",
    },
  ]);
});

test("a failed edit keeps dsh's error instead of an unapplied diff", () => {
  const { finished } = run(
    "edit",
    { file_path: "x", old_string: "a", new_string: "b" },
    "Error: old_string not found",
    "failed",
  );
  assert.equal(finished.status, "failed");
  assert.equal(finished.content[0].content.text, "Error: old_string not found");
});

test("str_replace_editor maps each command to read or a diff", () => {
  assert.equal(
    run("str_replace_editor", { command: "view", path: "a.ts" }, "x").started
      .kind,
    "read",
  );
  const insert = run("str_replace_editor", {
    command: "insert",
    path: "a.ts",
    insert_line: 1,
    new_str: "z",
  }, "ok");
  assert.equal(insert.started.title, "Insert into a.ts");
  assert.deepEqual(insert.finished.content, [{
    type: "diff",
    path: "/work/session/a.ts",
    oldText: "",
    newText: "z",
  }]);
  const replace = run("str_replace_editor", {
    command: "str_replace",
    path: "a.ts",
    old_str: "p",
    new_str: "q",
  }, "ok");
  assert.deepEqual(replace.finished.content[0].oldText, "p");
  const create = run("str_replace_editor", {
    command: "create",
    path: "b.ts",
    file_text: "new",
  }, "ok");
  assert.equal(create.started.title, "Write b.ts");
  assert.equal(create.finished.content[0].oldText, null);
});

test("search tools are labelled with their pattern or queries", () => {
  assert.deepEqual(
    [
      run("grep", { pattern: "Lasso", path: "AGENTS.md" }, "x").started.title,
      run("glob", { pattern: "**/*.md" }, "x").started.title,
    ],
    ['Grep "Lasso" in AGENTS.md', "Glob **/*.md"],
  );
  const search = run("web_search", { queries: ["a", "b"] }, "x").started;
  assert.equal(search.kind, "search");
  assert.equal(search.title, "Web search: a · b");
  assert.equal(
    run("web_fetch", { url: "https://example.com" }, "x").started.kind,
    "fetch",
  );
});

test("todo_write keeps its list for the todo renderer and publishes a plan", () => {
  const todos = [
    { content: "Inspect", status: "completed" },
    { content: "Edit", status: "in_progress" },
    { content: "Odd", status: "blocked" },
  ];
  const { started, extra } = run("todo_write", { todos }, "Updated todo list");
  assert.equal(started.title, "Update todos");
  assert.deepEqual(started.rawInput, { todos });
  assert.equal(extra.length, 1);
  assert.deepEqual(extra[0].params, {
    sessionId: "s",
    update: {
      sessionUpdate: "plan",
      entries: [
        { content: "Inspect", status: "completed", priority: "medium" },
        { content: "Edit", status: "in_progress", priority: "medium" },
        { content: "Odd", status: "pending", priority: "medium" },
      ],
    },
  });
  assert.equal(planOf({}), undefined);
});

test("unknown tools keep kind other with a readable title and finished calls are forgotten", () => {
  const { started, calls } = run("create_goal", { goal: "x" }, "ok");
  assert.equal(started.kind, "other");
  assert.equal(started.title, "Create goal");
  assert.equal(calls.size, 0);
});

test("every other message passes through unchanged", () => {
  const calls = new Map();
  for (
    const message of [
      { jsonrpc: "2.0", id: 1, result: { sessionId: "s" } },
      {
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: "s",
          update: {
            sessionUpdate: "agent_message_chunk",
            content: { type: "text", text: "hi" },
          },
        },
      },
      {
        jsonrpc: "2.0",
        id: 2,
        method: "session/request_permission",
        params: { toolCall: { toolCallId: "c" } },
      },
    ]
  ) {
    assert.deepEqual(presentMessage(message, calls, cwd), [message]);
  }
  // A result whose call this process never saw is forwarded as-is.
  const orphan = result("unknown", "x");
  assert.deepEqual(presentMessage(orphan, calls, cwd), [orphan]);
});

test("session requests establish the working directory", () => {
  assert.equal(
    requestCwd({ method: "session/new", params: { cwd: "/a" } }),
    "/a",
  );
  assert.equal(
    requestCwd({
      method: "session/resume",
      params: { cwd: "/b", sessionId: "s" },
    }),
    "/b",
  );
  assert.equal(
    requestCwd({ method: "session/new", params: { cwd: "relative" } }),
    undefined,
  );
  assert.equal(requestCwd({ method: "session/prompt", params: {} }), undefined);
});

test("fileText leaves unframed or unnumbered text alone", () => {
  assert.equal(fileText("plain"), "plain");
  assert.equal(
    fileText(
      "<path>/a</path>\n<type>file</type>\n<content>\nno numbers\n</content>",
    ),
    "no numbers",
  );
});

test("the profile patch routes web_fetch through the gateway", () => {
  assert.match(COWBOY_PATCH, /fetchProvider: deepseek-fetch-api/u);
  assert.match(COWBOY_PATCH, /searchProvider: deepseek-official/u);
  assert.match(
    COWBOY_PATCH,
    /baseURL: !!js process\.env\.DEEPSEEK_WEB_BASE_URL/u,
  );
});

test("the profile patch follows dsh's own home resolution", () => {
  assert.equal(
    dshHome({ DSH_HOME: "/isolated/dsh" }, "/home/u"),
    "/isolated/dsh",
  );
  assert.equal(
    dshHome({ DSH_HOME: "  " }, "/runtime/home"),
    "/runtime/home/.dsh",
  );
  assert.equal(dshHome({}, "/runtime/home"), "/runtime/home/.dsh");
  assert.equal(dshHome({ DSH_HOME: "relative" }, "/home/u"), undefined);
  assert.equal(dshHome({}, ""), undefined);
});
