import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { Connection } from "../../execution-connection.mjs";
import { Facade } from "./facade.mjs";
import { AcpProxy } from "./proxy.mjs";
import { fakeEndpoint } from "./test-endpoint.mjs";

import { canonicalArgsKey, MAX_WAITING, PlanGate, planToolPolicy } from "./plan-gate.mjs";

const makeTurn = (id = "t1") => ({ id, cancelled: false });

test("canonicalArgsKey deeply sorts nested keys, strips toolSummary and undefined, and rejects oversized/invalid inputs", () => {
  assert.equal(
    canonicalArgsKey({ b: 1, a: { y: 2, x: 3, drop: undefined }, toolSummary: "ignored" }),
    canonicalArgsKey({ a: { x: 3, y: 2 }, b: 1, toolSummary: "different" }),
  );
  assert.equal(canonicalArgsKey({}), "{}");
  assert.equal(canonicalArgsKey(null), undefined);
  assert.equal(canonicalArgsKey([]), undefined);
  assert.equal(canonicalArgsKey({ n: Number.NaN }), undefined);
  assert.equal(canonicalArgsKey({ big: "x".repeat(130 * 1024) }), undefined);
  let deep = {};
  let cursor = deep;
  for (let i = 0; i < 40; i++) {
    cursor.a = {};
    cursor = cursor.a;
  }
  assert.equal(canonicalArgsKey(deep), undefined);
});

test("planToolPolicy classifies reads/planning/stops, writes/subagent, and approval-gated tools", () => {
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
  for (const approve of ["RunCommand", "cowboy_execute", "memory_put", "suger_read"]) {
    assert.equal(planToolPolicy(approve), "approve");
  }
});

test("PlanGate refuses unapproved RunCommand and extension calls, allows exact single-use approval, and rejects replay", () => {
  const gate = new PlanGate();
  const turn = makeTurn();
  const call1 = { sessionId: "s", name: "RunCommand", args: { command: "git status" } };
  // Announced without approval -> refused.
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "c1",
      name: "mcp_cowboy_RunCommand",
      rawInput: { command: "git status", toolSummary: "Git status" },
      status: "in_progress",
    },
    turn,
  );
  assert.equal(
    gate.authorizeDispatch(call1, { mode: "plan", turn, ownerId: "s", managed: false }).allowed,
    false,
  );
  // Owner approves c1 -> single-use grant minted and consumed once.
  const prep = gate.preparePermission(
    "s",
    "s",
    { toolCallId: "c1", name: "mcp_cowboy_RunCommand" },
    turn,
  );
  assert.equal(prep.ok, true);
  assert.deepEqual(gate.recordPermissionReply(prep.token, true, turn, "plan"), {
    granted: true,
    forwardCancelled: false,
  });
  const ok = gate.authorizeDispatch(call1, { mode: "plan", turn, ownerId: "s", managed: false });
  assert.deepEqual(ok, { allowed: true, toolCallId: "c1" });
  assert.equal(gate.bind(call1), "c1");
  // Replay with the same args -> refused (single-use consumed).
  const replay = { sessionId: "s", name: "RunCommand", args: { command: "git status" } };
  assert.equal(
    gate.authorizeDispatch(replay, { mode: "plan", turn, ownerId: "s", managed: false }).allowed,
    false,
  );
});

test("PlanGate refuses concurrent identical calls when one is approved and another is not", () => {
  const gate = new PlanGate();
  const turn = makeTurn();
  const args = { command: "rm -rf /tmp/demo" };
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "c1",
      name: "mcp_cowboy_RunCommand",
      rawInput: args,
      status: "in_progress",
    },
    turn,
  );
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "c2",
      name: "mcp_cowboy_RunCommand",
      rawInput: args,
      status: "in_progress",
    },
    turn,
  );
  const prep = gate.preparePermission(
    "s",
    "s",
    { toolCallId: "c1", name: "mcp_cowboy_RunCommand", rawInput: args },
    turn,
  );
  assert.equal(prep.ok, true);
  assert.equal(gate.recordPermissionReply(prep.token, true, turn, "plan").granted, true);
  // Both c1 and c2 are active and undispatched -> ambiguous -> refused!
  const ambig = gate.authorizeDispatch(
    { sessionId: "s", name: "RunCommand", args },
    { mode: "plan", turn, ownerId: "s", managed: false },
  );
  assert.equal(ambig.allowed, false);
  assert.match(ambig.reason, /ambiguous/i);
  // Once c2 ends (failed/rejected), c1 becomes unambiguously correlatable.
  gate.trackUpdate(
    "s",
    { sessionUpdate: "tool_call_update", toolCallId: "c2", status: "failed" },
    turn,
  );
  const clear = gate.authorizeDispatch(
    { sessionId: "s", name: "RunCommand", args },
    { mode: "plan", turn, ownerId: "s", managed: false },
  );
  assert.deepEqual(clear, { allowed: true, toolCallId: "c1" });
});

test("PlanGate also treats an undispatched same-tool call with missing/ambiguous input as a collision", () => {
  const gate = new PlanGate();
  const turn = makeTurn();
  const args = { command: "echo hi" };
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "c1",
      name: "mcp_cowboy_RunCommand",
      rawInput: args,
      status: "in_progress",
    },
    turn,
  );
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "c2",
      name: "mcp_cowboy_RunCommand",
      status: "in_progress",
    },
    turn,
  );
  const prep = gate.preparePermission(
    "s",
    "s",
    { toolCallId: "c1", name: "mcp_cowboy_RunCommand", rawInput: args },
    turn,
  );
  assert.equal(prep.ok, true);
  gate.recordPermissionReply(prep.token, true, turn, "plan");
  assert.equal(
    gate.authorizeDispatch(
      { sessionId: "s", name: "RunCommand", args },
      { mode: "plan", turn, ownerId: "s", managed: false },
    ).allowed,
    false,
  );
});

test("PlanGate rejects argument tampering between announcement, permission request, and dispatch", () => {
  const gate = new PlanGate();
  const turn = makeTurn();
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "c1",
      name: "mcp_cowboy_RunCommand",
      rawInput: { command: "git status" },
      status: "in_progress",
    },
    turn,
  );
  // Permission request claims different args than announced tool_call -> refused & poisoned.
  assert.equal(
    gate.preparePermission(
      "s",
      "s",
      { toolCallId: "c1", name: "mcp_cowboy_RunCommand", rawInput: { command: "rm -rf /" } },
      turn,
    ).ok,
    false,
  );
  // Even dispatching the original announced args is now refused.
  assert.equal(
    gate.authorizeDispatch(
      { sessionId: "s", name: "RunCommand", args: { command: "git status" } },
      { mode: "plan", turn, ownerId: "s", managed: false },
    ).allowed,
    false,
  );
});

test("PlanGate expires grants on mode change, turn cancel, turn end, and terminal call update", () => {
  const gate = new PlanGate();
  const approveOne = (sessionId, ownerId, turn, id, args = { command: "ls" }) => {
    gate.trackUpdate(
      sessionId,
      {
        sessionUpdate: "tool_call",
        toolCallId: id,
        name: "mcp_cowboy_RunCommand",
        rawInput: args,
        status: "in_progress",
      },
      turn,
    );
    const prep = gate.preparePermission(
      sessionId,
      ownerId,
      { toolCallId: id, name: "mcp_cowboy_RunCommand", rawInput: args },
      turn,
    );
    assert.equal(prep.ok, true);
    assert.equal(gate.recordPermissionReply(prep.token, true, turn, "plan").granted, true);
    return prep.token;
  };

  // 1. Mode change expires parent and child grants.
  const t1 = makeTurn("t1");
  const childTurn = makeTurn("ct1");
  approveOne("s", "s", t1, "c-mode");
  approveOne("child", "s", childTurn, "cc-mode");
  gate.invalidateOwner("s", ["child"]);
  assert.equal(
    gate.authorizeDispatch(
      { sessionId: "s", name: "RunCommand", args: { command: "ls" } },
      { mode: "plan", turn: t1, ownerId: "s", managed: false },
    ).allowed,
    false,
  );
  assert.equal(
    gate.authorizeDispatch(
      { sessionId: "child", name: "RunCommand", args: { command: "ls" } },
      { mode: "plan", turn: childTurn, ownerId: "s", managed: false },
    ).allowed,
    false,
  );

  // 2. Stale permission reply arriving after mode change does not grant and signals cancel.
  const t2 = makeTurn("t2");
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "c-stale",
      name: "mcp_cowboy_RunCommand",
      rawInput: { command: "pwd" },
      status: "in_progress",
    },
    t2,
  );
  const prepStale = gate.preparePermission(
    "s",
    "s",
    { toolCallId: "c-stale", name: "mcp_cowboy_RunCommand", rawInput: { command: "pwd" } },
    t2,
  );
  gate.invalidateOwner("s", []);
  assert.deepEqual(gate.recordPermissionReply(prepStale.token, true, t2, "plan"), {
    granted: false,
    forwardCancelled: true,
  });

  // 3. Turn end / cancel expires grants.
  const t3 = makeTurn("t3");
  approveOne("s", "s", t3, "c-end");
  gate.clearTurn("s", t3);
  assert.equal(
    gate.authorizeDispatch(
      { sessionId: "s", name: "RunCommand", args: { command: "ls" } },
      { mode: "plan", turn: t3, ownerId: "s", managed: false },
    ).allowed,
    false,
  );

  // 4. Terminal tool_call_update revokes unused grant.
  const t4 = makeTurn("t4");
  approveOne("s", "s", t4, "c-term");
  gate.trackUpdate(
    "s",
    { sessionUpdate: "tool_call_update", toolCallId: "c-term", status: "failed" },
    t4,
  );
  assert.equal(
    gate.authorizeDispatch(
      { sessionId: "s", name: "RunCommand", args: { command: "ls" } },
      { mode: "plan", turn: t4, ownerId: "s", managed: false },
    ).allowed,
    false,
  );
});

test("PlanGate fails closed on waiting-table overflow for the remainder of the turn", () => {
  const gate = new PlanGate();
  const turn = makeTurn("t-overflow");
  for (let i = 0; i <= MAX_WAITING; i++) {
    gate.trackUpdate(
      "s",
      {
        sessionUpdate: "tool_call",
        toolCallId: `c-${i}`,
        name: "mcp_cowboy_RunCommand",
        rawInput: { command: `echo ${i}` },
        status: "in_progress",
      },
      turn,
    );
  }
  assert.equal(
    gate.preparePermission(
      "s",
      "s",
      {
        toolCallId: `c-${MAX_WAITING}`,
        name: "mcp_cowboy_RunCommand",
        rawInput: { command: `echo ${MAX_WAITING}` },
      },
      turn,
    ).ok,
    false,
  );
});

const rpcRequest = (id, method, params) => ({ jsonrpc: "2.0", id, method, params });

async function integratedHarness(t, { managed = false, mode = "plan" } = {}) {
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
    has: (name) => name === "cowboy_execute" || name === "memory_put",
    route: (name) =>
      name === "cowboy_execute" || name === "memory_put"
        ? { server: "cowboy", tool: name }
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

test("end-to-end R3: plan mode allows reads/UpdatePlan and refuses direct WriteFile, EditFile, unapproved RunCommand and extension calls at Facade dispatch", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    // Reads & UpdatePlan succeed directly without permission prompts.
    const rd = await h.facade.call("s", "ReadFile", { path: "hello.txt" });
    assert.equal(rd.isError, undefined);
    assert.match(rd.content[0].text, /hello world/);

    const ls = await h.facade.call("s", "ListDirectory", { path: "." });
    assert.equal(ls.isError, undefined);

    const sf = await h.facade.call("s", "SearchFiles", { pattern: "hello" });
    assert.equal(sf.isError, undefined);

    const up = await h.facade.call("s", "UpdatePlan", {
      plan: [{ step: "Inspect codebase", status: "in_progress" }],
    });
    assert.equal(up.isError, undefined);

    // WriteFile & EditFile refused unconditionally even without request_permission.
    const wf = await h.facade.call("s", "WriteFile", { path: "pwn.txt", content: "bad" });
    assert.equal(wf.isError, true);
    assert.match(wf.content[0].text, /plan mode/i);
    assert.throws(() => readFileSync(join(h.dir, "pwn.txt"), "utf8"));

    const ef = await h.facade.call("s", "EditFile", {
      path: "hello.txt",
      old_string: "hello",
      new_string: "pwned",
    });
    assert.equal(ef.isError, true);
    assert.match(ef.content[0].text, /plan mode/i);
    assert.equal(readFileSync(join(h.dir, "hello.txt"), "utf8"), "hello world\n");

    // Unapproved RunCommand & extension calls refused without executing anything.
    const rc = await h.facade.call("s", "RunCommand", { command: "echo SHOULD_NOT_RUN" });
    assert.equal(rc.isError, true);
    assert.match(rc.content[0].text, /approval|plan mode/i);

    const ext = await h.facade.call("s", "cowboy_execute", {
      action: "x",
      arguments: { b: 2, a: 1 },
    });
    assert.equal(ext.isError, true);
    assert.equal(h.extCalls.length, 0);

    // Owner approves one exact RunCommand and one exact extension call -> each runs once.
    const opts = [
      { optionId: "allow", name: "Allow", kind: "allow_once" },
      { optionId: "always", name: "Always", kind: "allow_always" },
      { optionId: "deny", name: "Deny", kind: "reject_once" },
    ];
    await h.proxy.fromAgent({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: "s",
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "rc-1",
          name: "mcp_cowboy_RunCommand",
          rawInput: { command: "echo PLAN_APPROVED_OK", toolSummary: "Check" },
          status: "in_progress",
        },
      },
    });
    // Permission request omitting rawInput falls back to announced tool_call by exact toolCallId.
    await h.proxy.fromAgent(
      rpcRequest(50, "session/request_permission", {
        sessionId: "s",
        toolCall: { toolCallId: "rc-1", name: "mcp_cowboy_RunCommand" },
        options: opts,
      }),
    );
    assert.equal(h.client.at(-1).method, "session/request_permission");
    assert.deepEqual(
      h.client.at(-1).params.options.map((o) => o.kind),
      ["allow_once", "reject_once"],
    );
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: 50,
      result: { outcome: { outcome: "selected", optionId: "allow" } },
    });
    const okRun = await h.facade.call("s", "RunCommand", {
      command: "echo PLAN_APPROVED_OK",
      toolSummary: "Different summary",
    });
    assert.equal(okRun.isError, undefined);
    assert.match(okRun.content[0].text, /PLAN_APPROVED_OK/);
    // Replay of same command is refused (single-use consumed).
    const replayRun = await h.facade.call("s", "RunCommand", { command: "echo PLAN_APPROVED_OK" });
    assert.equal(replayRun.isError, true);

    // Extension call with differently-ordered nested keys matches canonical grant and executes once.
    await h.proxy.fromAgent(
      rpcRequest(51, "session/request_permission", {
        sessionId: "s",
        toolCall: {
          toolCallId: "ext-1",
          name: "mcp_cowboy_cowboy_execute",
          rawInput: { action: "x", arguments: { a: 1, b: 2 } },
        },
        options: opts,
      }),
    );
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: 51,
      result: { outcome: { outcome: "selected", optionId: "allow" } },
    });
    const okExt = await h.facade.call("s", "cowboy_execute", {
      arguments: { b: 2, a: 1 },
      action: "x",
    });
    assert.equal(okExt.isError, undefined);
    assert.equal(okExt.content[0].text, "ext-ok:cowboy_execute");
    assert.equal(h.extCalls.length, 1);
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: unknown mode and switching mode fail closed at dispatch; stale permission reply after mode switch grants nothing", async (t) => {
  const h = await integratedHarness(t, { mode: undefined });
  try {
    // Unknown mode refuses WriteFile and unapproved RunCommand, allows ReadFile.
    assert.equal((await h.facade.call("s", "ReadFile", { path: "hello.txt" })).isError, undefined);
    assert.equal(
      (await h.facade.call("s", "WriteFile", { path: "x.txt", content: "1" })).isError,
      true,
    );
    assert.equal((await h.facade.call("s", "RunCommand", { command: "echo nope" })).isError, true);

    // Ask permission for RunCommand while in unknown mode, then start switching mode before owner replies.
    await h.proxy.fromAgent(
      rpcRequest(60, "session/request_permission", {
        sessionId: "s",
        toolCall: {
          toolCallId: "rc-sw",
          name: "mcp_cowboy_RunCommand",
          rawInput: { command: "echo stale" },
        },
        options: [{ optionId: "allow", kind: "allow_once" }],
      }),
    );
    await h.proxy.fromClient(
      rpcRequest(61, "session/set_mode", { sessionId: "s", modeId: "plan" }),
    );
    // Mode is now switching -> stale Allow reply must be cancelled and mint NO grant.
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: 60,
      result: { outcome: { outcome: "selected", optionId: "allow" } },
    });
    assert.deepEqual(h.agent.at(-1), {
      jsonrpc: "2.0",
      id: 60,
      result: { outcome: { outcome: "cancelled" } },
    });
    assert.equal((await h.facade.call("s", "RunCommand", { command: "echo stale" })).isError, true);

    // Confirm default mode -> WriteFile now succeeds!
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 61, result: {} });
    await h.proxy.fromClient(
      rpcRequest(62, "session/set_mode", { sessionId: "s", modeId: "default" }),
    );
    await h.proxy.fromAgent({ jsonrpc: "2.0", id: 62, result: {} });
    const wfOk = await h.facade.call("s", "WriteFile", { path: "ok.txt", content: "yes" });
    assert.equal(wfOk.isError, undefined);
    assert.equal(readFileSync(join(h.dir, "ok.txt"), "utf8"), "yes");
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
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: concurrent identical RunCommand calls fail closed until unambiguous, and WeakMap binds exact call result", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    const args = { command: "echo CONCURRENT_OK" };
    // Announce two identical RunCommand calls concurrently (c1 and c2).
    for (const toolCallId of ["c1", "c2"]) {
      await h.proxy.fromAgent({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: "s",
          update: {
            sessionUpdate: "tool_call",
            toolCallId,
            name: "mcp_cowboy_RunCommand",
            rawInput: args,
            status: "in_progress",
          },
        },
      });
    }
    // Owner approves ONLY c1.
    await h.proxy.fromAgent(
      rpcRequest(70, "session/request_permission", {
        sessionId: "s",
        toolCall: { toolCallId: "c1", name: "mcp_cowboy_RunCommand", rawInput: args },
        options: [{ optionId: "allow", kind: "allow_once" }],
      }),
    );
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: 70,
      result: { outcome: { outcome: "selected", optionId: "allow" } },
    });
    // While c2 is still active/undispatched, dispatch MUST refuse as ambiguous.
    const refused = await h.facade.call("s", "RunCommand", args);
    assert.equal(refused.isError, true);
    assert.match(refused.content[0].text, /ambiguous/i);

    // Once c2 ends (failed), c1 is unambiguously correlatable and succeeds once.
    await h.proxy.fromAgent({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: "s",
        update: { sessionUpdate: "tool_call_update", toolCallId: "c2", status: "failed" },
      },
    });
    const ok = await h.facade.call("s", "RunCommand", args);
    assert.equal(ok.isError, undefined);
    assert.match(ok.content[0].text, /CONCURRENT_OK/);
    // Terminal update for c1 receives the exact facade result via WeakMap binding.
    await h.proxy.fromAgent({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: "s",
        update: { sessionUpdate: "tool_call_update", toolCallId: "c1", status: "completed" },
      },
    });
    const last = h.client.at(-1);
    assert.equal(last.method, "session/update");
    assert.equal(last.params.update.toolCallId, "c1");
    assert.match(JSON.stringify(last.params.update.content), /CONCURRENT_OK/);
  } finally {
    await h.cleanup();
  }
});

test("end-to-end R3: session/cancel and turn end expire unused grants immediately", async (t) => {
  const h = await integratedHarness(t, { mode: "plan" });
  try {
    const args = { command: "echo SHOULD_EXPIRE" };
    await h.proxy.fromAgent({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: "s",
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "rc-exp",
          name: "mcp_cowboy_RunCommand",
          rawInput: args,
          status: "in_progress",
        },
      },
    });
    await h.proxy.fromAgent(
      rpcRequest(80, "session/request_permission", {
        sessionId: "s",
        toolCall: { toolCallId: "rc-exp", name: "mcp_cowboy_RunCommand", rawInput: args },
        options: [{ optionId: "allow", kind: "allow_once" }],
      }),
    );
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      id: 80,
      result: { outcome: { outcome: "selected", optionId: "allow" } },
    });
    // Cancel the turn -> grant must be cleared immediately and dispatch refused.
    await h.proxy.fromClient({
      jsonrpc: "2.0",
      method: "session/cancel",
      params: { sessionId: "s" },
    });
    const afterCancel = await h.facade.call("s", "RunCommand", args);
    assert.equal(afterCancel.isError, true);
    assert.equal(
      h.proxy.authorizeFacadeCall({ sessionId: "s", name: "RunCommand", args }).allowed,
      false,
    );
  } finally {
    await h.cleanup();
  }
});
