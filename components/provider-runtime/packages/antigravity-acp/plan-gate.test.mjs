import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Connection } from "../../execution-connection.mjs";
import { Facade } from "./facade.mjs";
import {
  argsDigest,
  isPlanEffectiveMode,
  isUnrestrictedMode,
  PlanGate,
  planToolPolicy,
} from "./plan-gate.mjs";
import { AcpProxy } from "./proxy.mjs";
import { fakeEndpoint } from "./test-endpoint.mjs";

const makeTurn = (id = "t1") => ({ id, cancelled: false, closed: false });
const rpcRequest = (id, method, params) => ({ jsonrpc: "2.0", id, method, params });

async function integratedHarness(t, options = {}) {
  const managed = options.managed ?? false;
  const hasMode = Object.hasOwn(options, "mode");
  const mode = hasMode ? options.mode : "plan";
  const base = realpathSync(mkdtempSync(join(tmpdir(), "cowboy-r3-plan-")));
  const dir = join(base, "workspace");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "hello.txt"), "hello world\n");
  const socket = join(base, "exec.sock");
  const endpoint = await fakeEndpoint(socket, dir, {
    managed: managed ? { profile: "read-only-v1" } : undefined,
  });
  let connection;
  t.after(async () => {
    connection?.close();
    await endpoint.close();
    rmSync(base, { recursive: true, force: true });
  });
  connection = await Connection.open(
    {
      schema: 1,
      endpoint: `unix:${socket}`,
      bearer_token: "a".repeat(64),
      binding: { schema: 1, environment: { id: "env" }, workspace: { cwd: dir } },
    },
    { clientName: "cowboy-antigravity-r3-test", requestTimeoutMs: 20000 },
  );
  const extCalls = [];
  let proxy;
  const facade = await Facade.create(connection, {
    managed,
    onResult: (call, result) => proxy?.toolResult(call, result),
    onProgress: (call, text) => proxy?.toolProgress(call, text),
  });
  facade.extensions = {
    has: (name) => name === "cowboy_execute" || name === "memory_put" || name === "folder_tool",
    route: (name) =>
      name === "cowboy_execute" || name === "memory_put"
        ? { server: "cowboy", tool: name }
        : name === "folder_tool"
          ? { server: "suger", tool: "write_note" }
          : undefined,
    tools: () => [],
    call: async (_conn, name, args) => {
      extCalls.push({ name, args });
      return { content: [{ type: "text", text: `ext-ok:${name}` }] };
    },
  };
  const agent = [];
  const client = [];
  const runtime = {
    facade,
    managed,
    evidence: {
      mark: async () => 1,
      settle: async () => ({ class: "ok", successes: 1, barrier: true, refusals: [] }),
    },
    memory: null,
    sessionCwd: (cwd) => cwd,
    checkMirror: () => {},
    projection: async () => "",
    newChannel: () => "ch-1",
    bindChannel: () => {},
    mcpServer: () => ({ name: "cowboy", command: "node", args: [] }),
  };
  proxy = new AcpProxy({
    runtime,
    agent: (msg) => agent.push(msg),
    client: (msg) => client.push(msg),
    log: () => {},
  });
  await proxy.runtime;
  await proxy.fromClient(rpcRequest(1, "initialize", { protocolVersion: 1 }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 1, result: { protocolVersion: 1 } });
  await proxy.fromClient(rpcRequest(2, "session/new", { cwd: dir, mcpServers: [] }));
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: 2,
    result: { sessionId: "s", ...(mode !== undefined ? { modes: { currentModeId: mode } } : {}) },
  });
  await proxy.fromClient(
    rpcRequest(3, "session/prompt", {
      sessionId: "s",
      prompt: [{ type: "text", text: "go" }],
      ...(managed ? { _meta: { "cowboy.dev/managedCall": { callId: "call-1" } } } : {}),
    }),
  );
  return {
    dir,
    facade,
    proxy,
    agent,
    client,
    extCalls,
    cleanup: async () => {
      await facade.endTurn("s").catch(() => {});
      connection.close();
      await endpoint.close();
      rmSync(base, { recursive: true, force: true });
    },
  };
}

test("closed unrestricted mode allowlist and tool classification (High #4)", () => {
  for (const ok of ["default", "accept-edits", "auto"]) {
    assert.equal(isUnrestrictedMode(ok), true);
    assert.equal(isPlanEffectiveMode(ok), false);
  }
  for (const restricted of ["plan", "switching", undefined, "", "readonly", "custom-mode", null]) {
    assert.equal(isUnrestrictedMode(restricted), false);
    assert.equal(isPlanEffectiveMode(restricted), true);
  }
  for (const allowed of [
    "ReadFile",
    "ListDirectory",
    "SearchFiles",
    "UpdatePlan",
    "StopCommand",
    "StopSubagent",
  ]) {
    assert.equal(planToolPolicy(allowed), "allow");
  }
  for (const refused of ["WriteFile", "EditFile", "Subagent"]) {
    assert.equal(planToolPolicy(refused), "refuse");
  }
  for (const approve of ["RunCommand", "cowboy_execute", "memory_put", "folder_tool"]) {
    assert.equal(planToolPolicy(approve), "approve");
  }
});

test("argsDigest distinguishes >128 KiB payloads and preserves toolSummary when requested (High #6, Medium #12)", () => {
  const bigA = { path: "big.txt", content: `${"a".repeat(150 * 1024)}A` };
  const bigB = { path: "big.txt", content: `${"a".repeat(150 * 1024)}B` };
  assert.notEqual(argsDigest(bigA), undefined);
  assert.notEqual(argsDigest(bigB), undefined);
  assert.notEqual(argsDigest(bigA), argsDigest(bigB));
  assert.equal(
    argsDigest({ b: 1, a: { y: 2, x: 3 }, toolSummary: "one" }, { stripSummary: true }),
    argsDigest({ a: { x: 3, y: 2 }, b: 1, toolSummary: "two" }, { stripSummary: true }),
  );
  assert.notEqual(
    argsDigest({ a: 1, toolSummary: "one" }, { stripSummary: false }),
    argsDigest({ a: 1, toolSummary: "two" }, { stripSummary: false }),
  );
  assert.equal(argsDigest(null), undefined);
  assert.equal(argsDigest({ n: Number.NaN }), undefined);
});

test("PlanGate.bind enforces turn lifetime and distinguishes large writes without swapping (Medium #12)", () => {
  const gate = new PlanGate();
  const t1 = makeTurn("t1");
  const big1 = { path: "f1.txt", content: `${"x".repeat(140 * 1024)}1` };
  const big2 = { path: "f2.txt", content: `${"x".repeat(140 * 1024)}2` };
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "w1",
      name: "mcp_cowboy_WriteFile",
      rawInput: big1,
      status: "in_progress",
    },
    t1,
  );
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "w2",
      name: "mcp_cowboy_WriteFile",
      rawInput: big2,
      status: "in_progress",
    },
    t1,
  );
  const call2 = { sessionId: "s", name: "WriteFile", args: big2 };
  const call1 = { sessionId: "s", name: "WriteFile", args: big1 };
  gate.admitCall(call2, t1);
  gate.admitCall(call1, t1);
  assert.equal(gate.bind(call2, t1), "w2");
  assert.equal(gate.bind(call1, t1), "w1");
  // Cancelling or closing t1 prevents late bind even if a newer turn t2 starts with same call.
  t1.cancelled = true;
  assert.equal(gate.bind(call1, t1), undefined);
  const t2 = makeTurn("t2");
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "w-next",
      name: "mcp_cowboy_WriteFile",
      rawInput: big1,
      status: "in_progress",
    },
    t2,
  );
  assert.equal(gate.bind(call1, t2), undefined);
});

test("end-to-end R3: plan mode allows reads/UpdatePlan, refuses writes/subagent, and prompts owner directly at Facade dispatch with exact payload (High #1, #6, #7)", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    // Reads & UpdatePlan succeed directly without permission prompts.
    assert.match(
      (await h.facade.call("s", "ReadFile", { path: "hello.txt" })).content[0].text,
      /hello world/,
    );
    assert.equal((await h.facade.call("s", "ListDirectory", { path: "." })).isError, undefined);
    assert.equal(
      (await h.facade.call("s", "SearchFiles", { pattern: "hello" })).isError,
      undefined,
    );
    assert.equal(
      (
        await h.facade.call("s", "UpdatePlan", {
          plan: [{ step: "Inspect", status: "in_progress" }],
        })
      ).isError,
      undefined,
    );
    assert.equal(h.client.filter((m) => m.method === "session/request_permission").length, 0);

    // WriteFile & EditFile refused unconditionally at dispatch (zero prompts, zero side effects).
    const wf = await h.facade.call("s", "WriteFile", { path: "pwn.txt", content: "bad" });
    assert.equal(wf.isError, true);
    assert.throws(() => readFileSync(join(h.dir, "pwn.txt"), "utf8"));
    const ef = await h.facade.call("s", "EditFile", {
      path: "hello.txt",
      old_string: "hello",
      new_string: "pwned",
    });
    assert.equal(ef.isError, true);
    assert.equal(readFileSync(join(h.dir, "hello.txt"), "utf8"), "hello world\n");

    // Direct RunCommand (even with NO prior agy request_permission or argument-less one)
    // prompts the owner directly with exact command and NO allow_always.
    const runPromise = h.facade.call("s", "RunCommand", { command: "echo FACADE_DIRECT_OK" });
    const req = h.client.at(-1);
    assert.equal(req.method, "session/request_permission");
    assert.equal(req.params.toolCall.title, "echo FACADE_DIRECT_OK");
    assert.deepEqual(req.params.toolCall.rawInput, { command: "echo FACADE_DIRECT_OK" });
    assert.deepEqual(
      req.params.options.map((o) => o.kind),
      ["allow_once", "reject_once"],
    );
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: req.id,
      result: { outcome: { outcome: "selected", optionId: "allow_once" } },
    });
    const runRes = await runPromise;
    assert.equal(runRes.isError, undefined);
    assert.match(runRes.content[0].text, /FACADE_DIRECT_OK/);

    // Extension call preserves toolSummary verbatim in owner approval prompt (High #6).
    const extInput = { note: "important", toolSummary: "PERSISTED_METADATA_VALUE" };
    const extPromise = h.facade.call("s", "folder_tool", extInput);
    const extReq = h.client.at(-1);
    assert.equal(extReq.method, "session/request_permission");
    assert.equal(extReq.params.toolCall.name, "mcp__suger__write_note");
    assert.deepEqual(extReq.params.toolCall.rawInput, extInput);
    // Mutating caller's original object after admission cannot alter executed payload.
    extInput.toolSummary = "MUTATED_AFTER_ADMISSION";
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: extReq.id,
      result: { outcome: { outcome: "selected", optionId: "allow_once" } },
    });
    const extRes = await extPromise;
    assert.equal(extRes.isError, undefined);
    assert.equal(h.extCalls.length, 1);
    assert.deepEqual(h.extCalls[0].args, {
      note: "important",
      toolSummary: "PERSISTED_METADATA_VALUE",
    });
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: two identical concurrent MCP RunCommand calls get isolated per-invocation approvals (High #1, #8)", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    const args = { command: "echo ISOLATED_ONCE" };
    const callA = h.facade.call("s", "RunCommand", args);
    const permA = h.client.at(-1);
    const callB = h.facade.call("s", "RunCommand", args);
    const permB = h.client.at(-1);
    assert.equal(permA.method, "session/request_permission");
    assert.equal(permB.method, "session/request_permission");
    assert.notEqual(permA.id, permB.id);
    assert.notEqual(permA.params.toolCall.toolCallId, permB.params.toolCall.toolCallId);

    // Reject callB FIRST, approve callA SECOND -> ONLY callA executes, callB is refused!
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: permB.id,
      result: { outcome: { outcome: "selected", optionId: "reject_once" } },
    });
    assert.equal((await callB).isError, true);

    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: permA.id,
      result: { outcome: { outcome: "selected", optionId: "allow_once" } },
    });
    const resA = await callA;
    assert.equal(resA.isError, undefined);
    assert.match(resA.content[0].text, /ISOLATED_ONCE/);

    // A third identical call MUST ask again and cannot reuse callA's approval.
    const callC = h.facade.call("s", "RunCommand", args);
    const permC = h.client.at(-1);
    assert.notEqual(permC.id, permA.id);
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: permC.id,
      result: { outcome: { outcome: "cancelled" } },
    });
    assert.equal((await callC).isError, true);
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: recycled agy IDs, stripped allow_always, web reads, ask_question, and tier redaction (High #2, Medium #9, #10, #14, #15)", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    const optsWithStanding = [
      {
        optionId: "allow",
        name: "Allow",
        kind: "allow_once",
        _meta: { antigravity: { tier: "pro" } },
      },
      { optionId: "always", name: "Always", kind: "allow_always" },
      { optionId: "deny", name: "Deny", kind: "reject_once" },
    ];
    // ask_question strips allow_always in plan mode and strips tier metadata from options.
    await h.proxy.fromAgent(
      rpcRequest(50, "session/request_permission", {
        sessionId: "s",
        toolCall: { toolCallId: "q1", name: "ask_question" },
        options: optsWithStanding,
      }),
    );
    const q1Req = h.client.at(-1);
    assert.equal(q1Req.method, "session/request_permission");
    assert.notEqual(q1Req.id, 50);
    assert.deepEqual(
      q1Req.params.options.map((o) => o.kind),
      ["allow_once", "reject_once"],
    );
    assert.equal(q1Req.params.options[0]._meta, undefined);

    // Selecting stripped `always` optionId MUST reply cancelled to agy (Medium #9).
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: q1Req.id,
      result: { outcome: { outcome: "selected", optionId: "always" } },
    });
    assert.deepEqual(h.agent.at(-1), {
      jsonrpc: "2.0",
      id: 50,
      result: { outcome: { outcome: "cancelled" } },
    });

    // Supported web reads (search_web, read_url_content, view_content_chunk) forward to owner
    // with allow_always stripped; recycled agy id 50 gets a fresh worker ID (High #2, Medium #14).
    for (const webTool of ["search_web", "read_url_content", "view_content_chunk"]) {
      await h.proxy.fromAgent(
        rpcRequest(50, "session/request_permission", {
          sessionId: "s",
          toolCall: { toolCallId: `w-${webTool}`, name: webTool, rawInput: { query: "test" } },
          options: optsWithStanding,
        }),
      );
      const wReq = h.client.at(-1);
      assert.equal(wReq.method, "session/request_permission");
      assert.notEqual(wReq.id, q1Req.id);
      assert.deepEqual(
        wReq.params.options.map((o) => o.kind),
        ["allow_once", "reject_once"],
      );
      await h.proxy.fromClient({
        jsonrpc: "2.0",
        id: wReq.id,
        result: { outcome: { outcome: "selected", optionId: "allow" } },
      });
      assert.deepEqual(h.agent.at(-1), {
        jsonrpc: "2.0",
        id: 50,
        result: { outcome: { outcome: "selected", optionId: "allow" } },
      });
    }

    // Stale standing option offered in default mode is cancelled if mode switches before reply (Medium #9).
    await h.proxy.fromClient(
      rpcRequest(100, "session/set_mode", { sessionId: "s", modeId: "default" }),
    );
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 100, result: {} });
    await h.proxy.fromAgent(
      rpcRequest(51, "session/request_permission", {
        sessionId: "s",
        toolCall: { toolCallId: "q2", name: "ask_question" },
        options: [{ optionId: "always", name: "Always", kind: "allow_always" }],
      }),
    );
    const defaultPermId = h.client.at(-1).id;
    await h.proxy.fromClient(
      rpcRequest(101, "session/set_mode", { sessionId: "s", modeId: "plan" }),
    );
    // Switching immediately cancels agy's pending request and notifies worker.
    assert.deepEqual(
      h.agent.find((m) => m.id === 51),
      {
        jsonrpc: "2.0",
        id: 51,
        result: { outcome: { outcome: "cancelled" } },
      },
    );
    assert.equal(
      h.client.some(
        (m) => m.method === "$/cancel_request" && m.params?.requestId === defaultPermId,
      ),
      true,
    );
    const agyCountBeforeLate = h.agent.length;
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: defaultPermId,
      result: { outcome: { outcome: "selected", optionId: "always" } },
    });
    assert.equal(h.agent.length, agyCountBeforeLate);
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: out-of-order switch replies, switch error invalidation, unknown modes, and reopen clear mode (High #3, #4)", async (t) => {
  const h = await integratedHarness(t, { mode: undefined });
  try {
    // Verify mode is genuinely undefined (not defaulted to "plan").
    assert.equal(h.proxy.effectiveMode("s"), undefined);
    assert.equal(
      (await h.facade.call("s", "WriteFile", { path: "u.txt", content: "1" })).isError,
      true,
    );

    // Unrecognized mode string fails closed!
    await h.proxy.fromAgent({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: "s",
        update: { sessionUpdate: "current_mode_update", currentModeId: "experimental-safe" },
      },
    });
    assert.equal(h.proxy.effectiveMode("s"), "experimental-safe");
    assert.equal(h.proxy.isPlanMode("s"), true);
    assert.equal(
      (await h.facade.call("s", "WriteFile", { path: "u2.txt", content: "1" })).isError,
      true,
    );

    // Out-of-order overlapping switches: `default` (req 61) then `plan` (req 62).
    // Even if `plan` (62) answers first and `default` (61) empty `{}` answers LAST,
    // writes MUST stay refused (High #3)!
    await h.proxy.fromClient(
      rpcRequest(61, "session/set_mode", { sessionId: "s", modeId: "default" }),
    );
    await h.proxy.fromClient(
      rpcRequest(62, "session/set_mode", { sessionId: "s", modeId: "plan" }),
    );
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 62, result: {} });
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 61, result: {} });
    assert.equal(h.proxy.effectiveMode("s"), undefined);
    assert.equal(
      (await h.facade.call("s", "WriteFile", { path: "u3.txt", content: "1" })).isError,
      true,
    );

    // Final switch error retires switching-era pending prompts (High #3).
    await h.proxy.fromClient(
      rpcRequest(63, "session/set_mode", { sessionId: "s", modeId: "default" }),
    );
    const duringSwitch = h.facade.call("s", "RunCommand", { command: "echo switch-err" });
    const swPermId = h.client.at(-1).id;
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 63, error: { code: -32000, message: "failed" } });
    assert.equal((await duringSwitch).isError, true);
    assert.equal(
      h.client.some((m) => m.method === "$/cancel_request" && m.params?.requestId === swPermId),
      true,
    );

    // Confirm default -> writes succeed; then session/load omitting `modes` clears default and fails closed (High #4).
    await h.proxy.fromClient(
      rpcRequest(64, "session/set_mode", { sessionId: "s", modeId: "default" }),
    );
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 64, result: {} });
    assert.equal(
      (await h.facade.call("s", "WriteFile", { path: "ok.txt", content: "yes" })).isError,
      undefined,
    );
    await h.proxy.fromClient(
      rpcRequest(65, "session/load", { sessionId: "s", cwd: h.dir, mcpServers: [] }),
    );
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 65, result: { sessionId: "s" } });
    assert.equal(h.proxy.effectiveMode("s"), undefined);
    assert.equal(
      (await h.facade.call("s", "WriteFile", { path: "after-load.txt", content: "no" })).isError,
      true,
    );
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: request-form session/cancel fences dispatch, retires permissions, and preserves reply (High #5, Medium #11)", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    const runPromise = h.facade.call("s", "RunCommand", { command: "echo CANCEL_REQ" });
    const permId = h.client.at(-1).id;
    assert.equal(h.client.at(-1).method, "session/request_permission");

    // Request-form session/cancel (with JSON-RPC id 77) immediately retires pending permission
    // and cancels the turn before prompt settlement.
    await h.proxy.fromClient(rpcRequest(77, "session/cancel", { sessionId: "s" }));
    assert.equal((await runPromise).isError, true);
    assert.equal(
      h.client.some((m) => m.method === "$/cancel_request" && m.params?.requestId === permId),
      true,
    );
    assert.equal(
      h.agent.some((m) => m.id === 77 && m.method === "session/cancel"),
      true,
    );
    // Reply to request-form session/cancel flows back to the worker cleanly.
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 77, result: {} });
    assert.deepEqual(h.client.at(-1), { jsonrpc: "2.0", id: 77, result: {} });

    // Late permission approval after cancel grants nothing.
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: permId,
      result: { outcome: { outcome: "selected", optionId: "allow_once" } },
    });
    assert.equal(
      (await h.facade.call("s", "RunCommand", { command: "echo AFTER_CANCEL" })).isError,
      true,
    );
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: child mode report does not revoke parent permissions (Medium #13)", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    // Register a mock child session owned by parent "s".
    h.proxy.subagents.children.set("child-1", { id: "child-1", parent: "s", state: "running" });
    const runPromise = h.facade.call("s", "RunCommand", { command: "echo PARENT_SURVIVES" });
    const permId = h.client.at(-1).id;
    // Child reports a native mode update -> must NOT invalidate parent's pending permission.
    await h.proxy.fromAgent({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: "child-1",
        update: { sessionUpdate: "current_mode_update", currentModeId: "default" },
      },
    });
    assert.equal(h.proxy.effectiveMode("s"), "plan");
    assert.equal(h.proxy.effectiveMode("child-1"), "plan");
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: permId,
      result: { outcome: { outcome: "selected", optionId: "allow_once" } },
    });
    const res = await runPromise;
    assert.equal(res.isError, undefined);
    assert.match(res.content[0].text, /PARENT_SURVIVES/);
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: managed read-only child still runs read-only RunCommand without owner permission prompts", async (t) => {
  const h = await integratedHarness(t, { managed: true, mode: undefined });
  try {
    const res = await h.facade.call("s", "RunCommand", { command: "echo MANAGED_RO_OK" });
    assert.equal(res.isError, undefined);
    assert.match(res.content[0].text, /MANAGED_RO_OK/);
    assert.equal(h.client.filter((m) => m.method === "session/request_permission").length, 0);
  } finally {
    await h.cleanup();
  }
});
