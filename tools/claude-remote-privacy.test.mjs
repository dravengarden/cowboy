import assert from "node:assert/strict";
import test from "node:test";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { WebSocketServer } from "ws";
import { Connection } from "../plugins/claude-code/runtime/connection.mjs";
import { startModBridge } from "../plugins/claude-code/runtime/mod-bridge.mjs";
import { WorkspaceTools } from "../plugins/claude-code/runtime/tools.mjs";

const privateDetails = {
  timezone: "Asia/Shanghai",
  country: "China",
  endpoint: "wss://private-executor.invalid",
};
const privateText = /Asia\/Shanghai|China|private-executor/;
const binding = {
  environment: { id: "fixture" },
  workspace: { cwd: "/workspace" },
};

test("Claude environment context does not spread executor placement metadata", async () => {
  async function context(info) {
    const tools = new WorkspaceTools({ info }, binding, "/unused/state.json");
    tools.begin = async () => ({
      platform: "/bin/bash\nLinux 6.12\n/bin/bash\n",
    });
    tools.instructions = async () => [];
    tools.gitStatus = async () => undefined;
    const { nonce: _nonce, ...result } = await tools.context();
    return result;
  }
  const ordinary = await context({});
  const regional = await context(privateDetails);
  assert.deepEqual(regional, ordinary);
  assert.match(regional.environment, /Primary working directory: \/workspace/);
  assert.match(regional.environment, /OS Version: Linux 6.12/);
  assert.doesNotMatch(JSON.stringify(regional), privateText);
});

test("executor errors retain private classification without publishing diagnostics", async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await once(server, "listening");
  t.after(() => {
    for (const socket of server.clients) socket.terminate();
    server.close();
  });
  const diagnostic = { code: -32000, message: JSON.stringify(privateDetails) };
  server.on("connection", (socket) => {
    socket.on("message", (bytes) => {
      const frame = JSON.parse(bytes.toString());
      if (frame.method === "initialized") return;
      socket.send(JSON.stringify(
        frame.method === "initialize"
          ? {
            id: frame.id,
            result: {
              sessionId: "fixture-session",
              environmentInfo: {
                executorVersion: "0.159.3",
                platformOs: "linux",
                cwd: "file:///workspace",
                shell: { path: "/bin/bash" },
                ...privateDetails,
              },
            },
          }
          : { id: frame.id, error: diagnostic },
      ));
    });
  });
  const connection = await Connection.open({
    endpoint: `ws://127.0.0.1:${server.address().port}`,
    bearer_token: "0".repeat(64),
    binding,
  });
  t.after(() => connection.close());
  await assert.rejects(connection.call("fs/readFile", {}), (error) => {
    assert.equal(error.message, "Target operation failed");
    assert.deepEqual(error.remote, diagnostic);
    return true;
  });
  const directory = await mkdtemp(join(tmpdir(), "claude-private-error-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const tools = new WorkspaceTools(
    connection,
    binding,
    join(directory, "state.json"),
  );
  const result = await tools.call("read", { file_path: "/workspace/file" });
  assert.equal(result.isError, true);
  assert.match(result.content[0].text, /Do not repeat a mutation/);
  assert.doesNotMatch(JSON.stringify(result), privateText);
});

function post(bridge, value) {
  return new Promise((resolve, reject) => {
    const req = request({
      socketPath: bridge.socketPath,
      path: "/tool",
      method: "POST",
      headers: { authorization: `Bearer ${bridge.token}` },
    }, (res) => {
      const chunks = [];
      res.on("data", (data) => chunks.push(data));
      res.on("end", () =>
        resolve({
          status: res.statusCode,
          body: JSON.parse(Buffer.concat(chunks)),
        }));
      res.on("error", reject);
    });
    req.on("error", reject);
    req.end(JSON.stringify(value));
  });
}

test("native Mods errors hide diagnostics and never replay an uncertain effect", async (t) => {
  let effects = 0;
  const bridge = await startModBridge({
    async nativeCall() {
      effects++;
      throw Object.assign(new Error(JSON.stringify(privateDetails)), {
        remote: privateDetails,
      });
    },
  });
  t.after(() => bridge.close());
  const call = {
    id: "toolu_private_failure",
    tool: "Write",
    input: { file_path: "file", content: "effect" },
  };
  const failed = await post(bridge, call);
  assert.equal(failed.status, 200);
  assert.match(failed.body.deny, /inspect state/);
  assert.doesNotMatch(JSON.stringify(failed.body), privateText);
  assert.equal((await post(bridge, call)).status, 409);
  assert.equal(effects, 1);
});

test("ordinary tool output is not rewritten to conceal user content", async (t) => {
  const output = JSON.stringify(privateDetails);
  const bridge = await startModBridge({
    async nativeCall() {
      return { result: { stdout: output, stderr: "", interrupted: false } };
    },
  });
  t.after(() => bridge.close());
  const result = await post(bridge, {
    id: "toolu_user_output",
    tool: "Bash",
    input: { command: "cat project-data.json" },
  });
  assert.equal(result.body.result.stdout, output);
});
