import assert from "node:assert/strict";
import { Duplex } from "node:stream";
import test from "node:test";
import { Facade, serveMcp, UNCERTAIN_REPORT, WRAP_UP_NOTE } from "./facade.mjs";
import { MANAGED_META_KEY } from "./managed.mjs";
import { AcpProxy, WRAP_UP_CAPABILITY_KEY } from "./proxy.mjs";

const request = (id, method, params = {}) => ({ jsonrpc: "2.0", id, method, params });
const flush = () => new Promise((resolve) => setImmediate(resolve));

async function openManagedSession(h, sessionId) {
  await h.proxy.fromClient(request(900, "session/new", { cwd: "/work/repo" }));
  await h.proxy.fromAgent({ jsonrpc: "2.0", id: 900, result: { sessionId } });
}

function makeMcpStream() {
  const sent = [];
  const stream = new Duplex({
    read() {},
    write(chunk, _enc, cb) {
      sent.push(chunk.toString("utf8"));
      cb();
    },
  });
  return { stream, sent };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeFakeConnection(handlers = {}) {
  const calls = [];
  return {
    calls,
    descriptor: { binding: { workspace: { cwd: "/work/repo" } } },
    info: { shell: { path: "/bin/sh" } },
    async call(method, params) {
      calls.push({ method, params });
      if (handlers[method]) return await handlers[method](params);
      if (method === "fs/canonicalize") return { path: params.path };
      return {};
    },
  };
}

function makeProxy({
  managed = false,
  memory = null,
  facade = null,
  evidence = null,
  backend = { class: "ok", successes: 1, barrier: true, refusals: [] },
} = {}) {
  const agent = [];
  const client = [];
  const turns = [];
  const channels = new Map();
  const realFacade = facade ?? {
    beginTurn: (id, opts) => turns.push(["begin", id, opts]),
    endTurn: async (id, opts) => turns.push(["end", id, opts]),
    detachTurn: (id, opts) => turns.push(["detach", id, opts]),
    hasOpenCommands: () => false,
  };
  const runtime = {
    managed,
    memory,
    facade: realFacade,
    sessionCwd: (cwd) => cwd,
    checkMirror: () => {},
    projection: async () => "PROJECTION",
    newChannel: () => `channel-${channels.size}`,
    bindChannel: (channel, sessionId) => channels.set(channel, sessionId),
    mcpServer: (channel) => ({
      name: "cowboy",
      command: "/node",
      args: ["/relay", "/sock", channel],
      env: [],
    }),
    evidence: evidence ?? {
      mark: async () => 10,
      settle: async (cursor, sessionId, stopReason) => ({
        ...backend,
        cursor,
        sessionId,
        stopReason,
      }),
    },
  };
  const proxy = new AcpProxy({
    agent: (message) => agent.push(message),
    client: (message) => client.push(message),
    runtime: Promise.resolve(runtime),
  });
  return { proxy, agent, client, turns, runtime };
}

test("initialize strips upstream steering/wrapUpSteering from both _meta surfaces and stamps wrapUpSteering only when unmanaged", async () => {
  const unmanaged = makeProxy({ managed: false });
  await unmanaged.proxy.fromClient(request(1, "initialize", { protocolVersion: 1 }));
  await unmanaged.proxy.fromAgent({
    jsonrpc: "2.0",
    id: 1,
    result: {
      protocolVersion: 1,
      _meta: {
        steering: { supported: true, idleBehavior: "promptRequired" },
        [WRAP_UP_CAPABILITY_KEY]: { supported: false },
        other: "keep",
      },
      agentCapabilities: {
        _meta: {
          steering: true,
          [WRAP_UP_CAPABILITY_KEY]: true,
          capOther: "keep",
        },
      },
    },
  });
  const uRes = unmanaged.client.at(-1).result;
  assert.equal(uRes._meta.steering, undefined);
  assert.deepEqual(uRes._meta[WRAP_UP_CAPABILITY_KEY], { v: 1, adapter: "cowboy-antigravity-acp" });
  assert.equal(uRes._meta.other, "keep");
  assert.equal(uRes.agentCapabilities._meta.steering, undefined);
  assert.equal(uRes.agentCapabilities._meta[WRAP_UP_CAPABILITY_KEY], undefined);
  assert.equal(uRes.agentCapabilities._meta.capOther, "keep");

  const managed = makeProxy({ managed: true });
  await openManagedSession(managed, "s2");
  await managed.proxy.fromClient(request(1, "initialize", { protocolVersion: 1 }));
  await managed.proxy.fromAgent({
    jsonrpc: "2.0",
    id: 1,
    result: {
      protocolVersion: 1,
      _meta: {
        steering: { supported: true },
        [WRAP_UP_CAPABILITY_KEY]: { supported: true },
      },
      agentCapabilities: {
        _meta: {
          steering: true,
          [WRAP_UP_CAPABILITY_KEY]: true,
        },
      },
    },
  });
  const mRes = managed.client.at(-1).result;
  assert.equal(mRes._meta?.steering, undefined);
  assert.equal(mRes._meta?.[WRAP_UP_CAPABILITY_KEY], undefined);
  assert.equal(mRes.agentCapabilities._meta?.steering, undefined);
  assert.equal(mRes.agentCapabilities._meta?.[WRAP_UP_CAPABILITY_KEY], undefined);
});

test("attemptNonce is stripped before agy and cowboy/wrapUpHint bypasses queue during reservedTurns", async () => {
  const recallGate = deferred();
  const memory = {
    recall: true,
    request: async () => await recallGate.promise,
    add: () => {},
    finish: async () => {},
  };
  const { proxy, agent, turns } = makeProxy({ memory });
  const promptPromise = proxy.fromClient(
    request(10, "session/prompt", {
      sessionId: "s1",
      prompt: [{ type: "text", text: "do work" }],
      _meta: { "cowboy.dev/attemptNonce": "nonce-1" },
    }),
  );
  await flush();
  await flush();
  assert.equal(agent.length, 0, "prompt is still suspended in memory.recall");

  proxy.fromClient({
    jsonrpc: "2.0",
    method: "cowboy/wrapUpHint",
    params: { sessionId: "s1", attemptNonce: "wrong-nonce" },
  });
  proxy.fromClient({
    jsonrpc: "2.0",
    method: "cowboy/wrapUpHint",
    params: { sessionId: "s1", attemptNonce: "nonce-1" },
  });

  recallGate.resolve(null);
  await promptPromise;
  assert.equal(agent.length, 1);
  assert.equal(agent[0].params._meta, undefined, "attemptNonce stripped and empty _meta removed");
  assert.equal(turns[0][0], "begin");
  assert.equal(
    turns[0][2].wrapUp.pending,
    true,
    "hint latched during reservedTurns carried to beginTurn",
  );
  assert.equal(turns[0][2].wrapUp.delivered, false);
});

test("cowboy/wrapUpHint ignores compacting, managed, cancelled, and structured turns", async () => {
  const compact = makeProxy();
  await compact.proxy.fromClient(
    request(1, "session/prompt", {
      sessionId: "s1",
      prompt: [{ type: "text", text: "/compact" }],
      _meta: { "cowboy.dev/attemptNonce": "n-compact" },
    }),
  );
  compact.proxy.fromClient({
    jsonrpc: "2.0",
    method: "cowboy/wrapUpHint",
    params: { sessionId: "s1", attemptNonce: "n-compact" },
  });
  assert.equal(compact.turns[0][2].wrapUp.pending, false);

  const managed = makeProxy({ managed: true });
  await openManagedSession(managed, "s2");
  await managed.proxy.fromClient(
    request(2, "session/prompt", {
      sessionId: "s2",
      prompt: [{ type: "text", text: "hi" }],
      _meta: {
        "cowboy.dev/attemptNonce": "n-managed",
        [MANAGED_META_KEY]: {
          callId: "c1",
          outputSchema: {
            type: "object",
            properties: { ok: { type: "boolean" } },
            required: ["ok"],
            additionalProperties: false,
          },
        },
      },
    }),
  );
  managed.proxy.fromClient({
    jsonrpc: "2.0",
    method: "cowboy/wrapUpHint",
    params: { sessionId: "s2", attemptNonce: "n-managed" },
  });
  assert.equal(managed.turns[0][2].wrapUp.pending, false);

  const cancelled = makeProxy();
  await cancelled.proxy.fromClient(
    request(3, "session/prompt", {
      sessionId: "s3",
      prompt: [{ type: "text", text: "hi" }],
      _meta: { "cowboy.dev/attemptNonce": "n-cancel" },
    }),
  );
  await cancelled.proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: "s3" },
  });
  cancelled.proxy.fromClient({
    jsonrpc: "2.0",
    method: "cowboy/wrapUpHint",
    params: { sessionId: "s3", attemptNonce: "n-cancel" },
  });
  assert.equal(cancelled.turns[0][2].wrapUp.pending, false);
});

test("native /compact (including split blocks and NBSP) skips skill expansion and memory recall", async () => {
  let recalled = 0;
  const memory = {
    recall: true,
    request: async () => {
      recalled += 1;
      return { text: "mem" };
    },
    add: () => {},
    finish: async () => {},
  };
  const { proxy, agent } = makeProxy({ memory });
  proxy.skills = new Map([["compact", { name: "compact", body: "EXPANDED" }]]);
  await proxy.fromClient(
    request(1, "session/prompt", {
      sessionId: "s1",
      prompt: [
        { type: "text", text: " /com" },
        { type: "text", text: "pact" },
      ],
    }),
  );
  assert.equal(recalled, 0);
  assert.equal(agent.length, 1);
  assert.equal(agent[0].params.prompt[0].text, " /com");
  assert.equal(agent[0].params.prompt[1].text, "pact");
});

test("cancelling mid-recall does not consume compaction.pending or mutate compacting", async () => {
  const recallGate = deferred();
  const memory = {
    recall: true,
    request: async () => await recallGate.promise,
    add: () => {},
    finish: async () => {},
  };
  const { proxy, agent, client } = makeProxy({ memory });
  proxy.compaction.opened("s1", "PROJECTION");
  proxy.compaction.settled("s1", "completed");
  assert.equal(proxy.compaction.pending.has("s1"), true);

  const promptPromise = proxy.fromClient(
    request(20, "session/prompt", {
      sessionId: "s1",
      prompt: [{ type: "text", text: "after compact" }],
    }),
  );
  await flush();
  await flush();
  await proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: "s1" },
  });
  recallGate.resolve({ text: "remembered" });
  await promptPromise;

  assert.equal(agent.length, 0);
  assert.equal(client.at(-1).id, 20);
  assert.equal(client.at(-1).result.stopReason, "cancelled");
  assert.equal(proxy.compaction.pending.has("s1"), true, "restatement preserved for next turn");
  assert.equal(proxy.compacting.has("s1"), false);
});

test("closeInput synchronously refuses new reservations and fences proxy.correct() before and after evidence.mark; attempt-2 gets fresh attemptCleanup", async () => {
  let recalled = 0;
  let markCount = 0;
  const correctMark = deferred();
  const closeHarness = makeProxy({
    managed: true,
    evidence: {
      mark: async () => {
        markCount += 1;
        if (markCount === 2) return await correctMark.promise;
        return markCount;
      },
      settle: async () => ({ class: "ok", successes: 1, barrier: true, refusals: [] }),
    },
  });

  await openManagedSession(closeHarness, "s1");
  const { proxy, agent, turns } = closeHarness;
  await proxy.fromClient(
    request(30, "session/prompt", {
      sessionId: "s1",
      prompt: [{ type: "text", text: "answer json" }],
      _meta: {
        [MANAGED_META_KEY]: {
          callId: "c1",
          outputSchema: {
            type: "object",
            properties: { ok: { type: "boolean" } },
            required: ["ok"],
            additionalProperties: false,
          },
        },
      },
    }),
  );
  await proxy.fromAgent({
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "s1",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "not json" } },
    },
  });
  const replyPromise = proxy.fromAgent({
    jsonrpc: "2.0",
    id: 30,
    result: { stopReason: "end_turn" },
  });
  await flush();
  await flush();
  proxy.closeInput();
  correctMark.resolve(2);
  await replyPromise;
  assert.equal(agent.filter((m) => m.method === "session/prompt").length, 1);

  const unmanaged = makeProxy({
    memory: {
      recall: true,
      request: async () => {
        recalled += 1;
        return null;
      },
      add: () => {},
      finish: async () => {},
    },
  });
  unmanaged.proxy.closeInput();
  await unmanaged.proxy.fromClient(
    request(31, "session/prompt", {
      sessionId: "s2",
      prompt: [{ type: "text", text: "late prompt" }],
    }),
  );
  assert.equal(recalled, 0);
  assert.equal(unmanaged.client.at(-1).id, 31);
  assert.ok(unmanaged.client.at(-1).error);

  const retryHarness = makeProxy({ managed: true });
  await openManagedSession(retryHarness, "s3");
  await retryHarness.proxy.fromClient(
    request(32, "session/prompt", {
      sessionId: "s3",
      prompt: [{ type: "text", text: "answer json" }],
      _meta: {
        [MANAGED_META_KEY]: {
          callId: "c2",
          outputSchema: {
            type: "object",
            properties: { ok: { type: "boolean" } },
            required: ["ok"],
            additionalProperties: false,
          },
        },
      },
    }),
  );
  await retryHarness.proxy.fromAgent({
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "s3",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "not json" } },
    },
  });
  await retryHarness.proxy.fromAgent({
    jsonrpc: "2.0",
    id: 32,
    result: { stopReason: "end_turn" },
  });
  const retryPrompts = retryHarness.agent.filter((m) => m.method === "session/prompt");
  assert.equal(retryPrompts.length, 2, "correction attempt-2 was dispatched");
  assert.equal(retryHarness.turns.filter((t) => t[0] === "end").length, 1);
  await retryHarness.proxy.fromAgent({
    jsonrpc: "2.0",
    id: retryPrompts[1].id,
    result: { stopReason: "end_turn" },
  });
  assert.equal(retryHarness.turns.filter((t) => t[0] === "end").length, 2);
  assert.ok(turns.length >= 1);
});

test("facade attaches WRAP_UP_NOTE non-mutatingly once after onResult (text, [text, image], image-only)", async () => {
  const presented = [];
  const conn = makeFakeConnection();
  const facade = new Facade(conn, {
    root: "/work/repo",
    shell: "/bin/sh",
    onResult: (_call, result) => presented.push(structuredClone(result)),
  });

  const originalMixed = {
    content: [
      { type: "text", text: "image metadata" },
      { type: "image", mimeType: "image/png", data: "AAAA" },
    ],
  };
  const wrapUp = { pending: true, delivered: false };
  facade.beginTurn("s1", { wrapUp });
  facade.answer = async () => originalMixed;
  const first = await facade.call("s1", "ReadFile", { path: "img.png" });

  assert.equal(presented[0].content[0].text, "image metadata");
  assert.equal(originalMixed.content[0].text, "image metadata");
  assert.equal(first.content.length, 2);
  assert.equal(
    first.content[0].text,
    `image metadata

${WRAP_UP_NOTE}`,
  );
  assert.deepEqual(first.content[1], { type: "image", mimeType: "image/png", data: "AAAA" });

  const second = await facade.call("s1", "ReadFile", { path: "img.png" });
  assert.equal(second.content[0].text, "image metadata");

  const originalImgOnly = {
    content: [{ type: "image", mimeType: "image/png", data: "BBBB" }],
  };
  facade.beginTurn("s2", { wrapUp: { pending: true, delivered: false } });
  facade.answer = async () => originalImgOnly;
  const imgResult = await facade.call("s2", "ReadFile", { path: "img2.png" });
  assert.equal(originalImgOnly.content.length, 1);
  assert.equal(imgResult.content.length, 2);
  assert.deepEqual(imgResult.content[1], { type: "text", text: WRAP_UP_NOTE });
});

test("ambiguousRelays suppresses wrap-up note across an intervening clean zero-MCP turn until verified relay close", async () => {
  const conn = makeFakeConnection();
  const facade = new Facade(conn, { root: "/work/repo", shell: "/bin/sh" });
  facade.answer = async () => ({ content: [{ type: "text", text: "ok" }] });

  const { stream: stream1, sent: out1 } = makeMcpStream();
  serveMcp(stream1, facade, () => "s1");

  facade.beginTurn("s1", { wrapUp: { pending: false, delivered: false } });
  await facade.endTurn("s1", { cancelled: true });
  facade.detachTurn("s1", { cancelled: true });

  facade.beginTurn("s1", { wrapUp: { pending: false, delivered: false } });
  await facade.endTurn("s1", { cancelled: false, errored: false });
  facade.detachTurn("s1", { cancelled: false, errored: false });

  const wrapUpC = { pending: true, delivered: false };
  facade.beginTurn("s1", { wrapUp: wrapUpC });

  stream1.push(
    `${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "ListDirectory", arguments: {} } })}
`,
  );
  await flush();
  await flush();
  const resp1 = JSON.parse(out1.join("").trim());
  assert.equal(resp1.result.content[0].text, "ok");
  assert.equal(wrapUpC.pending, true, "hint was NOT consumed while ambiguous relay 1 is open");
  assert.equal(wrapUpC.delivered, false);

  stream1.destroy();
  await flush();

  const { stream: stream2, sent: out2 } = makeMcpStream();
  serveMcp(stream2, facade, () => "s1");
  stream2.push(
    `${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "ListDirectory", arguments: {} } })}
`,
  );
  await flush();
  await flush();
  const resp2 = JSON.parse(out2.join("").trim());
  assert.equal(
    resp2.result.content[0].text,
    `ok

${WRAP_UP_NOTE}`,
  );
  assert.equal(wrapUpC.delivered, true);
  stream2.destroy();
});

test("root cancel forwards session/cancel immediately while attemptCleanup awaits stop window and reports UNCERTAIN_REPORT if unconfirmed", async () => {
  const stopGate = deferred();
  const uncertainCalls = [];
  const conn = makeFakeConnection({
    "process/terminate": async () => {
      await stopGate.promise;
      throw new Error("timeout waiting for exit");
    },
    [UNCERTAIN_REPORT]: async (params) => {
      uncertainCalls.push(params);
      return {};
    },
  });
  const facadeUncertain = [];
  const facade = new Facade(conn, {
    root: "/work/repo",
    shell: "/bin/sh",
    stopConfirmMs: 50,
    onUncertain: (sessionId, method) => facadeUncertain.push([sessionId, method]),
  });

  const { proxy, agent } = makeProxy({ facade });
  await proxy.fromClient(
    request(40, "session/prompt", {
      sessionId: "s1",
      prompt: [{ type: "text", text: "run slow command" }],
    }),
  );
  const sess = facade.session("s1");
  sess.commands.set("proc-1", {
    processId: "proc-1",
    command: "sleep 999",
    cleaning: 0,
    stopping: undefined,
    timer: undefined,
  });

  const cancelPromise = proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: "s1" },
  });
  assert.equal(
    agent.some((m) => m.method === "session/cancel" && m.params?.sessionId === "s1"),
    true,
    "session/cancel reached agy synchronously without waiting for command cleanup",
  );

  stopGate.resolve();
  await cancelPromise;
  assert.deepEqual(uncertainCalls, [{ method: "process/terminate" }]);
  assert.deepEqual(facadeUncertain, [["s1", "process/terminate"]]);
  facade.quiesce();
});

test("H2: synchronous session/cancel overtakes a prompt queued behind an earlier client request without ever writing it to agy", async () => {
  const newSessionGate = deferred();
  const h = makeProxy({ managed: false });
  h.runtime.projection = async () => {
    await newSessionGate.promise;
    return "PROJECTION";
  };

  // Start an async request that occupies proxy.queue.
  const firstReq = h.proxy.fromClient(request(100, "session/new", { cwd: "/work/repo" }));
  // Send session/prompt followed immediately by session/cancel BEFORE firstReq finishes!
  const promptReq = h.proxy.fromClient(
    request(101, "session/prompt", {
      sessionId: "s-queued",
      prompt: [{ type: "text", text: "must never reach agy" }],
      _meta: { "cowboy.dev/attemptNonce": "nonce-queued" },
    }),
  );
  await h.proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: "s-queued" },
  });

  newSessionGate.resolve();
  await firstReq;
  await promptReq;

  assert.equal(
    h.agent.some((m) => m.method === "session/prompt" && m.id === 101),
    false,
    "cancelled queued prompt must never be written to agy",
  );
  const cancelledReply = h.client.find((m) => m.id === 101);
  assert.deepEqual(cancelledReply?.result, { stopReason: "cancelled" });
});

test("H2: wrapUpHint arriving while prompt is queued behind an earlier request binds to that reserved turn", async () => {
  const gate = deferred();
  const h = makeProxy({ managed: false });
  await flush();
  h.runtime.projection = async () => {
    await gate.promise;
    return "PROJECTION";
  };
  const firstReq = h.proxy.fromClient(request(200, "session/new", { cwd: "/work/repo" }));
  const promptReq = h.proxy.fromClient(
    request(201, "session/prompt", {
      sessionId: "s-hint-q",
      prompt: [{ type: "text", text: "do work" }],
      _meta: { "cowboy.dev/attemptNonce": "nonce-hint-q" },
    }),
  );
  // Hint arrives synchronously while prompt is still waiting on this.queue!
  await h.proxy.fromClient({
    jsonrpc: "2.0",
    method: "cowboy/wrapUpHint",
    params: { sessionId: "s-hint-q", attemptNonce: "nonce-hint-q" },
  });
  gate.resolve();
  await firstReq;
  await promptReq;

  const turn = h.proxy.turns.get("s-hint-q");
  assert.equal(turn?.wrapUp.pending, true);
});

test("H4: quiesce closes open turns and rejects new MCP tool calls on already-accepted relays", async () => {
  const conn = makeFakeConnection({
    "fs/read": async () => ({ content: "should never run" }),
  });
  const facade = new Facade(conn, { root: "/work/repo", shell: "/bin/sh" });
  const { stream, sent } = makeMcpStream();
  serveMcp(stream, facade, () => "s-shut");
  facade.beginTurn("s-shut", { wrapUp: { pending: false, delivered: false } });

  facade.quiesce();
  assert.equal(facade.sessions.get("s-shut")?.turn, undefined);

  stream.push(
    `${JSON.stringify({ jsonrpc: "2.0", id: 7, method: "tools/call", params: { name: "ReadFile", arguments: { path: "a.txt" } } })}\n`,
  );
  await flush();
  await flush();

  assert.equal(conn.calls.length, 0, "no target RPC may start after quiesce");
  const resp = JSON.parse(sent.join("").trim());
  assert.equal(resp.result?.isError, true);
  assert.match(resp.result?.content?.[0]?.text ?? "", /shutting down/);
  stream.destroy();
});

test("M1: root Stop unconfirmed UNCERTAIN_REPORT still runs during quiesce drain and exposes remainingRootStopMs", async () => {
  const stopGate = deferred();
  const uncertainCalls = [];
  const conn = makeFakeConnection({
    "process/terminate": async () => {
      await stopGate.promise;
      throw new Error("still alive");
    },
    [UNCERTAIN_REPORT]: async (params) => {
      uncertainCalls.push(params);
      return {};
    },
  });
  const facade = new Facade(conn, {
    root: "/work/repo",
    shell: "/bin/sh",
    stopConfirmMs: 15_000,
  });
  facade.beginTurn("s-m1", { wrapUp: { pending: false, delivered: false } });
  facade.session("s-m1").commands.set("proc-m1", {
    processId: "proc-m1",
    command: "sleep 999",
    cleaning: 0,
    stopping: undefined,
    timer: undefined,
  });

  const endPromise = facade.endTurn("s-m1", { cancelled: true });
  // Even after proxy exit triggers quiesce(), drain budget reflects the active 15s root window.
  facade.quiesce();
  assert.ok(
    facade.remainingRootStopMs() > 10_000,
    `expected remainingRootStopMs > 10s, got ${facade.remainingRootStopMs()}`,
  );

  stopGate.resolve();
  await endPromise;
  assert.deepEqual(uncertainCalls, [{ method: "process/terminate" }]);
  assert.equal(facade.remainingRootStopMs(), 0);
});

test("M2: unsettled ReadFile call during turn end marks open MCP relay ambiguous until socket close", async () => {
  const readGate = deferred();
  const conn = makeFakeConnection();
  const facade = new Facade(conn, { root: "/work/repo", shell: "/bin/sh" });
  let slowOnce = true;
  facade.answer = async () => {
    if (slowOnce) {
      slowOnce = false;
      await readGate.promise;
    }
    return { content: [{ type: "text", text: "read-ok" }] };
  };

  const { stream: relay1, sent: out1 } = makeMcpStream();
  serveMcp(relay1, facade, () => "s-m2");

  // Turn 1 starts an ordinary ReadFile; agy ends the turn before ReadFile replies!
  facade.beginTurn("s-m2", { wrapUp: { pending: false, delivered: false } });
  relay1.push(
    `${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "ReadFile", arguments: { path: "x" } } })}\n`,
  );
  await flush();
  await facade.endTurn("s-m2", { cancelled: false, errored: false });
  facade.detachTurn("s-m2", { cancelled: false, errored: false });
  readGate.resolve();
  await flush();
  await flush();

  // Turn 2 on the same relay MUST NOT receive WRAP_UP_NOTE because relay1 is ambiguous!
  const wrapUp2 = { pending: true, delivered: false };
  facade.beginTurn("s-m2", { wrapUp: wrapUp2 });
  out1.length = 0;
  relay1.push(
    `${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "ReadFile", arguments: { path: "y" } } })}\n`,
  );
  await flush();
  await flush();
  const resp2 = JSON.parse(out1.join("").trim());
  assert.equal(resp2.result.content[0].text, "read-ok");
  assert.equal(wrapUp2.delivered, false);

  relay1.destroy();
  await flush();
});
