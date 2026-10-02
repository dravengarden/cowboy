import assert from "node:assert/strict";
import { lstat } from "node:fs/promises";
import { request } from "node:http";
import { dirname } from "node:path";
import test from "node:test";
import { startModBridge } from "./mod-bridge.mjs";

function post(bridge, path, value, token = bridge.token) {
  return new Promise((resolve, reject) => {
    const req = request({
      socketPath: bridge.socketPath,
      path,
      method: "POST",
      headers: { authorization: `Bearer ${token}` },
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

test("private socket authenticates before dispatch and refuses duplicate effects", async (t) => {
  const calls = [];
  let finish;
  const bridge = await startModBridge({
    async nativeCall(name, input) {
      calls.push({ name, input });
      return await new Promise((resolve) => finish = resolve);
    },
  });
  t.after(() => bridge.close());
  assert.equal((await lstat(dirname(bridge.socketPath))).mode & 0o777, 0o700);
  assert.equal((await lstat(bridge.socketPath)).mode & 0o777, 0o600);
  const call = { id: "toolu_once", tool: "Bash", input: { command: "effect" } };
  assert.equal((await post(bridge, "/tool", call, "wrong")).status, 403);
  assert.equal(
    (await post(bridge, "/tool", { ...call, tool: "Agent" })).status,
    400,
  );
  assert.equal((await post(bridge, "/ready", {})).body.ready, true);
  assert.equal(calls.length, 0);
  const pending = post(bridge, "/tool", call);
  while (!finish) await new Promise((resolve) => setImmediate(resolve));
  assert.equal((await post(bridge, "/tool", call)).status, 409);
  finish({ result: { stdout: "done", stderr: "", interrupted: false } });
  assert.equal((await pending).body.result.stdout, "done");
  assert.equal((await post(bridge, "/tool", call)).status, 409);
  assert.deepEqual(calls, [{ name: "Bash", input: call.input }]);
});

test("a failed target result is denied and its admitted identity cannot replay", async () => {
  let effects = 0;
  const bridge = await startModBridge({
    async nativeCall() {
      effects++;
      throw new Error("lost result after effect");
    },
  });
  try {
    const call = {
      id: "toolu_lost",
      tool: "Write",
      input: { file_path: "file", content: "effect" },
    };
    const failed = await post(bridge, "/tool", call);
    assert.equal(failed.status, 200);
    assert.match(failed.body.deny, /inspect state/);
    assert.equal((await post(bridge, "/tool", call)).status, 409);
    assert.equal(effects, 1);
  } finally {
    await bridge.close();
  }
  await assert.rejects(lstat(bridge.socketPath), { code: "ENOENT" });
});

test("long operations are observed under one identity without repeating admission", async (t) => {
  let finish;
  let effects = 0;
  const bridge = await startModBridge({
    nativeCall() {
      effects++;
      return new Promise((resolve) => finish = resolve);
    },
  }, { waitMs: 5 });
  t.after(() => bridge.close());
  const call = {
    id: "toolu_long",
    tool: "Bash",
    input: { command: "long effect" },
  };
  assert.deepEqual(await post(bridge, "/tool", call), {
    status: 202,
    body: { pending: call.id },
  });
  assert.equal((await post(bridge, "/tool", call)).status, 409);
  assert.equal((await post(bridge, "/result", { id: call.id })).status, 202);
  finish({ result: { stdout: "done", stderr: "", interrupted: false } });
  assert.equal(
    (await post(bridge, "/result", { id: call.id })).body.result.stdout,
    "done",
  );
  assert.equal((await post(bridge, "/result", { id: call.id })).status, 409);
  assert.equal(effects, 1);
});
