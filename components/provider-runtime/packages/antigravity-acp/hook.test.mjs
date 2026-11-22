import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { ALLOWED_TOOLS, DENIAL_REASON, decide, URL_DENIAL_REASON, WEB_TOOLS } from "./hook.mjs";

const hook = fileURLToPath(new URL("./hook.mjs", import.meta.url));
const input = (name, args = {}) => JSON.stringify({ toolCall: { name, args } });

test("only the cowboy server's tools and ask_question are allowed", () => {
  for (const name of ALLOWED_TOOLS) assert.deepEqual(decide(input(name)), { decision: "allow" });
  assert.equal(ALLOWED_TOOLS.length, 9);
  // The session's extensions are tools of the same server.
  for (const name of [
    "mcp_cowboy_cowboy_read",
    "mcp_cowboy_memory_search",
    "mcp_cowboy_suger_read",
  ]) {
    assert.deepEqual(decide(input(name)), { decision: "allow" }, name);
  }
  for (const name of [
    "read_file",
    "run_command",
    "write_file",
    "invoke_subagent",
    "schedule",
    "swarm",
    "run_workflow",
    "read_browser_page",
    "open_browser_url",
    "browser_subagent",
    "mcp_cowboy_ReadFile ",
    "mcp_other_ReadFile",
    "mcp_cowboy_",
    "mcp_cowboy_a b",
    "mcp_cowboy_x.y",
    "ReadFile",
    "constructor",
    "__proto__",
    "",
  ]) {
    assert.equal(decide(input(name)).decision, "deny", name);
  }
});

test("a denied built-in with a Cowboy alternative names it", () => {
  for (const name of ["invoke_subagent", "define_subagent", "swarm", "run_workflow"]) {
    const { decision, reason } = decide(input(name));
    assert.equal(decision, "deny", name);
    assert.match(reason, /calls\.start/, name);
  }
  assert.match(
    decide(input("schedule", { DurationSeconds: 60 })).reason,
    /scheduled Cowboy message/,
  );
  assert.match(decide(input("manage_task")).reason, /RunCommand with\s+wait_seconds/);
  const delegation = decide(input("invoke_subagent")).reason;
  assert.match(delegation, /review\.start/);
  assert.match(delegation, /queue\.submit/);
  assert.match(delegation, /notify:true and end your turn/);
  assert.match(delegation, /mcp_cowboy_Subagent/);
  // The denial is shown to subagents too: it must not send them to a tool that
  // refuses them.
  assert.match(delegation, /A subagent\s+cannot start one: it does the work itself/);
  assert.match(decide(input("execute_url")).reason, /search_web/);
  assert.equal(decide(input("read_file")).reason, DENIAL_REASON);
  // An inherited property name is no reason.
  assert.equal(decide(input("constructor")).reason, DENIAL_REASON);
});

test("the network-only web built-ins are allowed", () => {
  assert.deepEqual(WEB_TOOLS, ["search_web", "read_url_content", "view_content_chunk"]);
  assert.deepEqual(decide(input("search_web", { query: "acp spec" })), { decision: "allow" });
  assert.deepEqual(decide(input("view_content_chunk", { document_id: "d1", position: 2 })), {
    decision: "allow",
  });
  for (const args of [
    { Url: "https://example.com/docs" },
    { url: "http://example.com" },
    { Url: "https://example.com", toolSummary: "Read the docs" },
  ]) {
    assert.deepEqual(decide(input("read_url_content", args)), { decision: "allow" }, args);
  }
});

test("a URL read names only http(s) URLs", () => {
  for (const args of [
    {},
    undefined,
    { Url: "file:///etc/passwd" },
    { Url: "ftp://example.com" },
    { Url: "javascript:alert(1)" },
    { Url: "not a url" },
    { Url: 7 },
    { Url: "https://example.com", BaseUrl: "file:///home" },
    { Url: `https://example.com/${"a".repeat(9000)}` },
  ]) {
    const decision = decide(input("read_url_content", args));
    assert.equal(decision.decision, "deny", JSON.stringify(args));
    assert.equal(decision.reason, URL_DENIAL_REASON);
  }
});

test("malformed input is denied", () => {
  for (const value of [
    undefined,
    "",
    "{",
    "null",
    "[]",
    '{"toolCall":null}',
    '{"toolCall":{"name":7}}',
    '{"toolCall":{"name":["mcp_cowboy_ReadFile"]}}',
    '{"name":"mcp_cowboy_ReadFile"}',
    `${input("mcp_cowboy_ReadFile")}${" ".repeat(1024 * 1024)}`,
  ]) {
    const decision = decide(value);
    assert.equal(decision.decision, "deny");
    assert.match(decision.reason, /cowboy/);
  }
});

test("the hook process prints one decision and fails closed", () => {
  const run = (stdin) => {
    const result = spawnSync(process.execPath, [hook], {
      input: stdin,
      env: { ...process.env, COWBOY_HOOK_PROFILE: "ordinary" },
      encoding: "utf8",
    });
    assert.equal(result.status, 0);
    return JSON.parse(result.stdout).decision;
  };
  assert.equal(run(input("mcp_cowboy_RunCommand")), "allow");
  assert.equal(run(input("run_command")), "deny");
  assert.equal(run("not json"), "deny");
  assert.equal(run(""), "deny");
  assert.equal(run(Buffer.alloc(2 * 1024 * 1024, 0x20)), "deny");
});
