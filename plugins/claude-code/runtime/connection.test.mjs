import assert from "node:assert/strict";
import test from "node:test";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { WebSocketServer } from "ws";
import { Connection } from "./connection.mjs";
import { WorkspaceTools } from "./tools.mjs";

async function fixture(t) {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await once(server, "listening");
  const state = {
    writes: 0,
    connections: 0,
    resumes: [],
    session: "original",
    terminated: [],
  };
  server.on("connection", (socket) => {
    state.connections++;
    socket.on("message", (bytes) => {
      const frame = JSON.parse(bytes.toString());
      if (frame.method === "initialized") return;
      if (frame.method === "initialize") {
        state.resumes.push(frame.params.resumeSessionId);
        socket.send(JSON.stringify({
          id: frame.id,
          result: {
            sessionId: state.session,
            environmentInfo: {
              executorVersion: "0.159.3",
              platformOs: "linux",
              cwd: "file:///target",
              shell: { path: "/bin/bash" },
            },
          },
        }));
      } else if (frame.method === "fs/writeFile") {
        state.writes++;
        socket.terminate(); // Effect committed; acknowledgement lost.
      } else if (frame.method === "process/terminate") {
        state.terminated.push(frame.params.processId);
        socket.send(JSON.stringify({ id: frame.id, result: {} }));
      } else if (frame.method === "process/read") {
        socket.send(
          JSON.stringify({
            id: frame.id,
            result: { closed: true, chunks: [] },
          }),
        );
      } else {
        socket.send(JSON.stringify({ id: frame.id, result: { ok: true } }));
      }
    });
  });
  t.after(() => {
    for (const socket of server.clients) socket.terminate();
    server.close();
  });
  const connection = await Connection.open({
    endpoint: `ws://127.0.0.1:${server.address().port}`,
    bearer_token: "0".repeat(64),
    binding: { workspace: { cwd: "/target" } },
  });
  t.after(() => connection.close());
  return { state, connection };
}

test("new calls resume the same executor after lost replies without replaying effects", async (t) => {
  const { state, connection } = await fixture(t);
  await assert.rejects(connection.call("fs/writeFile", {}), /no replay/);
  assert.equal(state.writes, 1);
  const results = await Promise.all([
    connection.call("fs/readFile", {}),
    connection.call("environment/info", {}),
  ]);
  assert.deepEqual(results, [{ ok: true }, { ok: true }]);
  assert.equal(state.connections, 2);
  assert.deepEqual(state.resumes, [undefined, "original"]);
  assert.equal(state.writes, 1);
});

test("reconnection refuses a replacement executor session", async (t) => {
  const { state, connection } = await fixture(t);
  await assert.rejects(connection.call("fs/writeFile", {}), /no replay/);
  state.session = "replacement";
  await assert.rejects(connection.call("fs/readFile", {}), /binding|identity/);
  assert.equal(state.writes, 1);
});

test("explicit shutdown cannot reopen the execution connection", async (t) => {
  const { state, connection } = await fixture(t);
  connection.close();
  await assert.rejects(connection.call("fs/readFile", {}), /no replay/);
  assert.equal(state.connections, 1);
});

test("retained cancellation resumes after a new call restores the connection", async (t) => {
  const { state, connection } = await fixture(t);
  const directory = await mkdtemp(join(tmpdir(), "claude-reconnect-cancel-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const tools = new WorkspaceTools(connection, {
    environment: { id: "same-target" },
    workspace: { cwd: "/target" },
  }, join(directory, "state.json"));
  await assert.rejects(connection.call("fs/writeFile", {}), /no replay/);
  tools.state.jobs.original = { cancelRequested: true };
  tools.scheduleCancellations();
  assert.equal(tools.cancelTimer, undefined);
  await connection.call("fs/readFile", {});
  const deadline = Date.now() + 3000;
  while (tools.state.jobs.original.cancelRequested && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.equal(tools.state.jobs.original.cancelRequested, false);
  assert.deepEqual(state.terminated, ["original"]);
  assert.equal(state.writes, 1);
});
