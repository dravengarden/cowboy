import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { ALLOWED_TOOLS, decide } from "./hook.mjs";

const hook = fileURLToPath(new URL("./hook.mjs", import.meta.url));
const input = (name) => JSON.stringify({ toolCall: { name, args: {} } });

test("only the cowboy server's tools and ask_question are allowed", () => {
  for (const name of ALLOWED_TOOLS) assert.deepEqual(decide(input(name)), { decision: "allow" });
  assert.equal(ALLOWED_TOOLS.length, 8);
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
    "search_web",
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
    const result = spawnSync(process.execPath, [hook], { input: stdin, encoding: "utf8" });
    assert.equal(result.status, 0);
    return JSON.parse(result.stdout).decision;
  };
  assert.equal(run(input("mcp_cowboy_RunCommand")), "allow");
  assert.equal(run(input("run_command")), "deny");
  assert.equal(run("not json"), "deny");
  assert.equal(run(""), "deny");
  assert.equal(run(Buffer.alloc(2 * 1024 * 1024, 0x20)), "deny");
});
