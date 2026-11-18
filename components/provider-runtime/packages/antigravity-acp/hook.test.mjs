import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { ALLOWED_TOOLS, decide } from "./hook.mjs";

const hook = fileURLToPath(new URL("./hook.mjs", import.meta.url));
const input = (name) => JSON.stringify({ toolCall: { name, args: {} } });

test("only the exact facade tools and ask_question are allowed", () => {
  for (const name of ALLOWED_TOOLS) assert.deepEqual(decide(input(name)), { decision: "allow" });
  assert.equal(ALLOWED_TOOLS.length, 8);
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
    "mcp_cowboy_readfile",
    "mcp_other_ReadFile",
    "ReadFile",
    "mcp_cowboy_Delete",
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
