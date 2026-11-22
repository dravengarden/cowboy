import assert from "node:assert/strict";
import test from "node:test";
import { once } from "node:events";
import { WebSocketServer } from "ws";
import {
  probeExecutionCli,
  verifyExecutionCli,
} from "../components/provider-runtime/execution-probe.mjs";
import { codexMcpBindings, bindingConfiguration } from "../plugins/codex/runtime/bindings.mjs";

import { Connection, executionUrl } from "../components/provider-runtime/execution-connection.mjs";
import { createServer } from "node:http";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const identity = {
  command: "/nix/store/test-cli/bin/cowboy",
  sha256: "a".repeat(64),
  version: "1.0.0",
};
const capabilities = {
  schema: 1,
  descriptor_schema: 1,
  relay_schema: 1,
  grant_mcp: true,
  transport: "stdio_tool_only_v1",
};
const plan = {
  mcp: {
    schema: 1,
    command: "/nix/store/test-cli/bin/cowboy",
    bindings: [{ name: "suger", descriptor: "/target/private/binding.json" }],
  },
};

async function fixture(t, mode = "success") {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await once(server, "listening");
  t.after(() => {
    for (const socket of server.clients) socket.terminate();
    server.close();
  });
  const calls = [];
  let reads = 0;
  let initializations = 0;
  server.on("connection", (socket) =>
    socket.on("message", (bytes) => {
      const frame = JSON.parse(bytes.toString());
      if (frame.method === "initialized") return;
      calls.push(frame);
      const reply = (result) => socket.send(JSON.stringify({ id: frame.id, result }));
      if (frame.method === "initialize") {
        initializations++;
        const pin =
          mode === "malformed-identity"
            ? { ...identity, sha256: "bad" }
            : mode === "changed-identity" && initializations > 1
              ? { ...identity, sha256: "b".repeat(64) }
              : mode === "reordered-identity" && initializations > 1
                ? { version: identity.version, sha256: identity.sha256, command: identity.command }
                : identity;
        return reply({
          ...(mode === "legacy" ? {} : { cowboyAgentCli: pin }),
          sessionId: "probe-session",
          environmentInfo: {
            executorVersion: "0.159.3",
            platformOs: "linux",
            cwd: "file:///target",
            shell: { path: "/bin/bash" },
          },
          ...(mode === "managed" ? { cowboyManaged: { profile: "read_only_v1" } } : {}),
        });
      }
      if (frame.method === "process/start") {
        if (mode === "lost-start") return socket.terminate();
        return reply({ processId: frame.params.processId });
      }
      if (frame.method === "process/terminate") return reply({});
      if (frame.method === "process/read") {
        if (mode === "hang-read") return;
        if (mode === "late-read") {
          setTimeout(() => reply({ closed: true, exited: true, exitCode: 0, chunks: [] }), 300);
          return;
        }
        const index = reads++;
        if (index === 0) {
          const output = mode === "oversize" ? "x".repeat(4097) : JSON.stringify(capabilities);
          return reply({
            closed: true,
            exited: true,
            exitCode: 0,
            chunks: [{ seq: 1, stream: "stdout", chunk: Buffer.from(output).toString("base64") }],
          });
        }
        if (mode === "repeated")
          return reply({ chunks: [{ seq: 1, stream: "stdout", chunk: "eA==" }] });
        return reply({ closed: true, exited: true, exitCode: 0, chunks: [] });
      }
      throw new Error("Unexpected probe method");
    }),
  );
  return {
    calls,
    descriptor: {
      schema: 1,
      endpoint: `ws://127.0.0.1:${server.address().port}/`,
      bearer_token: "a".repeat(64),
      binding: { environment: { id: "target-env" }, workspace: { cwd: "/target" } },
    },
  };
}

test("Codex probes the bound target and emits only target placement and a context placeholder", async (t) => {
  const { calls, descriptor } = await fixture(t);
  const entries = await codexMcpBindings(plan, descriptor, ["cowboy_binding_suger"]);
  const start = calls.find((call) => call.method === "process/start");
  assert.deepEqual(start.params.argv, [
    "/nix/store/test-cli/bin/cowboy",
    "mcp",
    "--binding-capabilities",
  ]);
  assert.equal(start.params.cwd, "file:///target");
  assert.deepEqual(start.params.env, {});
  assert.equal(calls.filter((call) => call.method === "process/start").length, 1);
  assert.equal(calls.filter((call) => call.method === "process/terminate").length, 0);
  const configuration = bindingConfiguration(entries, descriptor);
  assert.match(configuration[1], /^mcp_servers.cowboy_binding_suger_2=/);
  assert.match(
    configuration[1],
    /environment_id="target-env",cwd="\/target",env=\{COWBOY_CALL_CONTEXT=""\}/,
  );
  assert.match(configuration[1], /args=\["mcp","--binding","\/target\/private\/binding.json"\]/);
  assert.equal(configuration.join("").includes(descriptor.bearer_token), false);
});

for (const mode of ["oversize", "repeated", "lost-start"]) {
  test(`probe ${mode} cleans up its original ID without replay`, async (t) => {
    const { calls, descriptor } = await fixture(t, mode);
    await assert.rejects(() =>
      probeExecutionCli(descriptor, [
        "/nix/store/test-cli/bin/cowboy",
        "mcp",
        "--binding-capabilities",
      ]),
    );
    const starts = calls.filter((call) => call.method === "process/start");
    const stops = calls.filter((call) => call.method === "process/terminate");
    assert.equal(starts.length, 1);
    assert.equal(stops.length, 1);
    assert.equal(stops[0].params.processId, starts[0].params.processId);
    if (mode === "lost-start")
      assert.equal(
        calls.filter((call) => call.method === "initialize")[1].params.resumeSessionId,
        "probe-session",
      );
  });
}

test("an enforced managed environment never starts the probe", async (t) => {
  const { calls, descriptor } = await fixture(t, "managed");
  await assert.rejects(
    () => probeExecutionCli(descriptor, ["/nix/store/test-cli/bin/cowboy"]),
    /Managed/,
  );
  assert.equal(
    calls.some((call) => call.method === "process/start"),
    false,
  );
});

test("Codex refuses local fallback and managed binding plans before probing", async () => {
  const probe = async () => {
    throw new Error("unexpected probe");
  };
  assert.deepEqual(await codexMcpBindings(null, null, [], probe), []);
  await assert.rejects(() => codexMcpBindings(plan, null, [], probe), /ordinary bound execution/);
  await assert.rejects(
    () => codexMcpBindings(plan, { binding: { managed: {} } }, [], probe),
    /ordinary bound execution/,
  );
  assert.deepEqual(bindingConfiguration([], null), []);
  assert.throws(
    () => bindingConfiguration([{ name: "suger", argv: ["/command"] }], null),
    /target placement/,
  );
});

test("a stalled probe read times out and terminates the original process", {
  timeout: 20000,
}, async (t) => {
  const { calls, descriptor } = await fixture(t, "hang-read");
  await assert.rejects(() => probeExecutionCli(descriptor, ["/nix/store/test-cli/bin/cowboy"]));
  const start = calls.find((call) => call.method === "process/start");
  const stops = calls.filter((call) => call.method === "process/terminate");
  assert.equal(stops.length, 1);
  assert.equal(stops[0].params.processId, start.params.processId);
});

for (const mode of ["legacy", "malformed-identity"]) {
  test(`binding probe refuses ${mode} before starting a process`, async (t) => {
    const { descriptor, calls } = await fixture(t, mode);
    await assert.rejects(() =>
      probeExecutionCli(descriptor, [identity.command, "mcp", "--binding-capabilities"]),
    );
    assert.equal(
      calls.some((call) => call.method === "process/start"),
      false,
    );
  });
}

test("native launch rechecks the exact target CLI without a second process probe", async (t) => {
  const { descriptor, calls } = await fixture(t);
  assert.equal(await verifyExecutionCli(descriptor, identity.command), identity.command);
  await assert.rejects(() => verifyExecutionCli(descriptor, "/nix/store/other/bin/cowboy"));
  assert.equal(
    calls.some((call) => call.method === "process/start"),
    false,
  );
});

for (const mode of ["changed-identity", "reordered-identity"]) {
  test(`reconnect checks CLI identity: ${mode}`, async (t) => {
    const { descriptor } = await fixture(t, mode);
    const connection = await Connection.open(descriptor);
    t.after(() => connection.close());
    const socket = connection.socket;
    socket.terminate();
    await once(socket, "close");
    if (mode === "changed-identity")
      await assert.rejects(() => connection.ensureConnected(), /identity changed/);
    else {
      await connection.ensureConnected();
      assert.equal(connection.assertAgentCli(identity.command), identity.command);
    }
  });
}

test("a unix: endpoint connects over the named socket with the bearer token", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "cowboy-exec-"));
  const path = join(directory, "exec.sock");
  const server = createServer();
  const wss = new WebSocketServer({ noServer: true });
  const seen = [];
  server.on("upgrade", (request, socket, head) => {
    seen.push(request.headers.authorization);
    if (request.headers.authorization !== `Bearer ${"b".repeat(64)}`) {
      socket.end("HTTP/1.1 401 Unauthorized\r\n\r\n");
      return;
    }
    wss.handleUpgrade(request, socket, head, (ws) => {
      ws.on("message", (bytes) => {
        const frame = JSON.parse(bytes.toString());
        if (frame.id === undefined) return;
        const result =
          frame.method === "initialize"
            ? {
                sessionId: "unix-session",
                environmentInfo: {
                  executorVersion: "0.159.3",
                  platformOs: "linux",
                  cwd: "file:///target",
                  shell: { path: "/bin/bash" },
                },
              }
            : { echoed: frame.method };
        ws.send(JSON.stringify({ id: frame.id, result }));
      });
    });
  });
  server.listen(path);
  await once(server, "listening");
  t.after(() => {
    for (const socket of wss.clients) socket.terminate();
    server.close();
    rmSync(directory, { recursive: true, force: true });
  });
  const descriptor = {
    schema: 1,
    endpoint: `unix:${path}`,
    bearer_token: "b".repeat(64),
    binding: { environment: { id: "target-env" }, workspace: { cwd: "/target" } },
  };
  const connection = await Connection.open(descriptor);
  t.after(() => connection.close());
  assert.equal(connection.sessionId, "unix-session");
  assert.deepEqual(await connection.call("fs/readFile", {}), { echoed: "fs/readFile" });
  await assert.rejects(() =>
    Connection.open({ ...descriptor, bearer_token: "c".repeat(64) }, { requestTimeoutMs: 2000 }),
  );
  assert.deepEqual(seen, [`Bearer ${"b".repeat(64)}`, `Bearer ${"c".repeat(64)}`]);
});

test("a request's own timeout rejects only it and ignores its late reply", async (t) => {
  const { descriptor } = await fixture(t, "late-read");
  const connection = await Connection.open(descriptor, { requestTimeoutMs: 5000 });
  t.after(() => connection.close());
  await assert.rejects(
    connection.call("process/read", { processId: "p" }, { timeoutMs: 50 }),
    /timed out/,
  );
  assert.equal(connection.abandoned.size, 1);
  // The late reply arrives and is consumed; the connection stays usable.
  await new Promise((resolve) => setTimeout(resolve, 400));
  assert.equal(connection.closed, false);
  assert.equal(connection.abandoned.size, 0);
  assert.deepEqual(await connection.call("process/terminate", { processId: "p" }), {});
});

test("execution endpoints keep ws:// unchanged and validate unix: paths", () => {
  assert.equal(executionUrl("ws://127.0.0.1:9/"), "ws://127.0.0.1:9/");
  assert.equal(executionUrl("unix:/run/cowboy-exec.sock"), "ws+unix:/run/cowboy-exec.sock:/");
  for (const bad of ["unix:relative.sock", "unix:/a:b", "unix:/a\0b", 7]) {
    assert.throws(() => executionUrl(bad), /Invalid execution endpoint/);
  }
});
