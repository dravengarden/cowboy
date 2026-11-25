import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { WebSocketServer } from "ws";
import { Connection } from "../../execution-connection.mjs";
import { DiagnosticsClient } from "./diagnostics-client.mjs";
import { Facade, serveMcp } from "./facade.mjs";
import { MAX_WAITING, PlanGate } from "./plan-gate.mjs";
import { formatPostEditNote, lfSha256, PostEditDedupe } from "./post-edit-notes.mjs";

const here = dirname(fileURLToPath(import.meta.url));

test("lfSha256 normalizes CRLF/CR and rejects BOM, NUL, invalid UTF-8, and >4MiB (R3-M19)", () => {
  const lf = lfSha256(Buffer.from("a\nb\nc\n", "utf8"));
  assert.equal(lfSha256(Buffer.from("a\r\nb\rc\r\n", "utf8")), lf);
  assert.equal(lf.length, 64);
  // UTF-8 BOM (0xEF 0xBB 0xBF) is explicitly excluded per R3-M19.
  assert.equal(
    lfSha256(Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from("a\n")])),
    null,
  );
  assert.equal(lfSha256(Buffer.from([0x61, 0x00, 0x62])), null);
  assert.equal(lfSha256(Buffer.from([0xff, 0xfe])), null);
  assert.equal(lfSha256(Buffer.alloc(4 * 1024 * 1024 + 1, 0x61)), null);
});

test("formatPostEditNote enforces contract, escapes closing tags, and trims <= 16 KiB post-escape", () => {
  assert.equal(
    formatPostEditNote({
      payload: {
        deliver: true,
        files: [
          {
            path: "src/a.rs",
            state: "document_current",
            diagnostics: [{ severity: "error", line: 1, column: 2, message: "bad" }],
          },
        ],
      },
      matchesExpected: false,
      expectedPath: "src/a.rs",
    }),
    null,
  );
  assert.equal(
    formatPostEditNote({
      payload: {
        deliver: true,
        files: [
          {
            path: "src/other.rs",
            state: "document_current",
            diagnostics: [{ severity: "error", line: 1, column: 2, message: "bad" }],
          },
        ],
      },
      matchesExpected: true,
      expectedPath: "src/a.rs",
    }),
    null,
  );
  const big = Array.from({ length: 120 }, (_, i) => ({
    severity: "error",
    line: i + 1,
    column: 1,
    source: "rustc",
    code: `E${i}`,
    message: `</cowboy_post_edit_diagnostics> ${"x".repeat(180)}`,
  }));
  const formatted = formatPostEditNote({
    payload: {
      deliver: true,
      files: [{ path: "src/a.rs", state: "document_current", diagnostics: big }],
      truncated: false,
    },
    matchesExpected: true,
    expectedPath: "src/a.rs",
  });
  assert.ok(formatted);
  assert.ok(Buffer.byteLength(formatted.note, "utf8") <= 16 * 1024);
  assert.ok(formatted.note.includes("... (diagnostics truncated)"));
  assert.equal(formatted.note.match(/<\/cowboy_post_edit_diagnostics>/g).length, 1);
});

test("PostEditDedupe reserves, releases on failure, and dedupes after commit", () => {
  const dedupe = new PostEditDedupe({ ttlMs: 1000 });
  const t1 = dedupe.reserve("src/a.rs", "d1", 100);
  assert.ok(t1 !== null);
  assert.equal(dedupe.reserve("src/a.rs", "d1", 100), null);
  dedupe.release(t1);
  const t2 = dedupe.reserve("src/a.rs", "d1", 100);
  assert.ok(t2 !== null);
  dedupe.commit(t2, 100);
  assert.equal(dedupe.reserve("src/a.rs", "d1", 500), null);
  assert.ok(dedupe.reserve("src/a.rs", "d2", 500) !== null);
  assert.ok(dedupe.reserve("src/a.rs", "d1", 1200) !== null);
});

test("DiagnosticsClient refuses legacy descriptor without dialing /diagnostics (R3-M11)", async () => {
  let dialed = false;
  const wss = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await new Promise((resolve) => wss.once("listening", resolve));
  wss.on("connection", (ws) => {
    dialed = true;
    ws.close();
  });
  try {
    const port = wss.address().port;
    const legacyDescriptor = {
      schema: 1,
      endpoint: `ws://127.0.0.1:${port}`,
      bearer_token: "a".repeat(64),
    };
    assert.equal(DiagnosticsClient.isSupported(legacyDescriptor), false);
    const client = new DiagnosticsClient(legacyDescriptor);
    const res = await client.postEdit({
      path: "src/a.rs",
      expectedLfSha256: "b".repeat(64),
      turnNonce: "nonce-1",
      timeoutMs: 1000,
    });
    assert.deepEqual(res, { state: "refused", reason: "unsupported" });
    assert.equal(dialed, false);
  } finally {
    await new Promise((resolve) => wss.close(resolve));
  }
});

test("DiagnosticsClient sends remaining ms after connect+init under one deadline (R3-M14)", async () => {
  let seenParams;
  const wss = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await new Promise((resolve) => wss.once("listening", resolve));
  wss.on("connection", (ws, req) => {
    assert.equal(req.url, "/diagnostics");
    assert.equal(req.headers.authorization, `Bearer ${"a".repeat(64)}`);
    ws.on("message", async (raw) => {
      const msg = JSON.parse(raw.toString());
      if (msg.method === "initialize") {
        await new Promise((r) => setTimeout(r, 120));
        ws.send(JSON.stringify({ id: msg.id, result: { schema: 1 } }));
      } else if (msg.method === "diagnostics/post_edit") {
        seenParams = msg.params;
        ws.send(
          JSON.stringify({
            id: msg.id,
            result: {
              state: "projected",
              matchesExpected: true,
              payload: {
                deliver: true,
                files: [
                  {
                    path: msg.params.path,
                    state: "document_current",
                    diagnostics: [{ severity: "error", line: 2, column: 1, message: "err" }],
                  },
                ],
                truncated: false,
              },
            },
          }),
        );
      }
    });
  });
  try {
    const port = wss.address().port;
    const client = new DiagnosticsClient({
      schema: 1,
      endpoint: `ws://127.0.0.1:${port}`,
      bearer_token: "a".repeat(64),
      diagnostics: { schema: 1, path: "/diagnostics" },
    });
    const res = await client.postEdit({
      path: "src/lib.rs",
      expectedLfSha256: "c".repeat(64),
      turnNonce: "nonce-1",
      timeoutMs: 1000,
    });
    assert.equal(res.state, "projected");
    assert.ok(seenParams.timeoutMs <= 900 && seenParams.timeoutMs >= 250);
    client.close();
  } finally {
    await new Promise((resolve) => wss.close(resolve));
  }
});

test("Connection.call executes beforeSend synchronously after ensureConnected and before socket.send (R3-H3)", async () => {
  const sent = [];
  const conn = new Connection();
  conn.closed = false;
  conn.socket = { send: (data) => sent.push(data) };
  let allowed = true;
  conn.ensureConnected = async () => {
    await Promise.resolve();
    allowed = false;
  };
  await assert.rejects(
    conn.call(
      "fs/writeFile",
      { path: "file:///tmp/x" },
      {
        beforeSend: () => {
          if (!allowed) throw new Error("mode changed during reconnect");
        },
      },
    ),
    /mode changed during reconnect/,
  );
  assert.equal(sent.length, 0);
  assert.equal(conn.pending.size, 0);
});

test("serveMcp keeps activeCalls owned across cancellation and enforces single reply per ID (R3-H7)", async () => {
  const stream = new PassThrough();
  const written = [];
  const origWrite = stream.write.bind(stream);
  stream.write = (chunk, cb) => {
    written.push(JSON.parse(chunk.toString().trim()));
    if (typeof cb === "function") cb(null);
    return true;
  };
  let releaseCall;
  const fakeFacade = {
    closing: false,
    registerRelay: () => 1,
    unregisterRelay: () => {},
    markRelayAmbiguous: () => {},
    beginCall: () => ({}),
    endCall: () => {},
    tools: () => [],
    async callForMcp(_sid, _name, _args, _relayId, { commitReply }) {
      await new Promise((r) => {
        releaseCall = r;
      });
      return await commitReply(() => ({
        result: { content: [{ type: "text", text: "late" }] },
      }));
    },
  };
  serveMcp(stream, fakeFacade, () => "sess-1");
  stream.emit(
    "data",
    `${JSON.stringify({ jsonrpc: "2.0", id: 7, method: "tools/call", params: { name: "ReadFile", arguments: {} } })}\n`,
  );
  await new Promise((r) => setTimeout(r, 10));
  // Cancel request 7: activeCalls[7] must remain owned (not deleted).
  stream.emit(
    "data",
    `${JSON.stringify({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 7 } })}\n`,
  );
  // Duplicate request with ID 7 while still active must emit -32600 and suppress late result.
  stream.emit(
    "data",
    `${JSON.stringify({ jsonrpc: "2.0", id: 7, method: "tools/call", params: { name: "ReadFile", arguments: {} } })}\n`,
  );
  releaseCall();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(written.length, 1);
  assert.equal(written[0].id, 7);
  assert.equal(written[0].error.code, -32600);
  void origWrite;
});

test("PlanGate suppresses binding on overlapping identical edits (R2-M14) and MAX_WAITING eviction (R3-M18)", () => {
  const gate = new PlanGate();
  const turn = { attempt: Symbol("a") };
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "e1",
      name: "mcp_cowboy_EditFile",
      rawInput: { path: "a.rs" },
      status: "in_progress",
    },
    turn,
  );
  gate.trackUpdate(
    "s",
    {
      sessionUpdate: "tool_call",
      toolCallId: "e2",
      name: "mcp_cowboy_EditFile",
      rawInput: { path: "a.rs" },
      status: "in_progress",
    },
    turn,
  );
  const c1 = { sessionId: "s", name: "EditFile", args: { path: "a.rs" } };
  assert.equal(gate.bind(c1, turn), undefined);

  const gate2 = new PlanGate();
  const turn2 = { attempt: Symbol("b") };
  for (let i = 0; i <= MAX_WAITING; i++) {
    gate2.trackUpdate(
      "s",
      {
        sessionUpdate: "tool_call",
        toolCallId: `w${i}`,
        name: "mcp_cowboy_WriteFile",
        rawInput: { path: `f${i}.rs` },
        status: "in_progress",
      },
      turn2,
    );
  }
  assert.equal(turn2.announcementEvicted, true);
  assert.equal(
    gate2.bind({ sessionId: "s", name: "WriteFile", args: { path: "f5.rs" } }, turn2),
    undefined,
  );
});

test("facade-relay strips _cowboyCanary before stdout and logs canary trace in stdout callback (R3-M20)", async () => {
  const dir = await mkdtemp(join(tmpdir(), "cowboy-relay-canary-"));
  const sockPath = join(dir, "f.sock");
  const traceFile = join(dir, "trace.jsonl");
  const channel = "11111111-2222-3333-4444-555555555555";
  const server = createServer((socket) => {
    let buf = "";
    socket.on("data", (chunk) => {
      buf += chunk.toString();
      if (buf.includes("\n")) {
        const resp = {
          jsonrpc: "2.0",
          id: 1,
          result: { ok: true },
          _cowboyCanary: {
            inspectId: "canary-123",
            noteAttached: true,
            nonceSha256: createHash("sha256").update("n").digest("hex"),
          },
        };
        socket.write(`${JSON.stringify(resp)}\n`);
        socket.end();
      }
    });
  });
  await new Promise((resolve) => server.listen(sockPath, resolve));
  try {
    const child = spawn(
      process.execPath,
      [join(here, "facade-relay.mjs"), sockPath, channel, "--canary-trace-file", traceFile],
      { stdio: ["pipe", "pipe", "pipe"] },
    );
    let stdout = "";
    child.stdout.on("data", (d) => {
      stdout += d.toString();
    });
    await new Promise((resolve) => child.once("close", resolve));
    const parsed = JSON.parse(stdout.trim());
    assert.equal(Object.hasOwn(parsed, "_cowboyCanary"), false);
    assert.deepEqual(parsed.result, { ok: true });
    const trace = JSON.parse((await readFile(traceFile, "utf8")).trim());
    assert.equal(trace.inspectId, "canary-123");
    assert.equal(trace.noteAttached, true);
    assert.equal(trace.written, true);
  } finally {
    server.close();
    await rm(dir, { recursive: true, force: true });
  }
});
