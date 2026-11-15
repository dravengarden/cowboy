import assert from "node:assert/strict";
import { lstat, mkdtemp, rm, readdir, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { request } from "node:http";
import { dirname } from "node:path";
import test from "node:test";
import { startModBridge } from "./mod-bridge.mjs";
import { SpentCalls } from "./mod-bridge-ledger.mjs";

function post(bridge, path, value, token = bridge.token) {
  return new Promise((resolve, reject) => {
    const req = request(
      {
        socketPath: bridge.socketPath,
        path,
        method: "POST",
        headers: { authorization: `Bearer ${token}` },
      },
      (res) => {
        const chunks = [];
        res.on("data", (data) => chunks.push(data));
        res.on("end", () =>
          resolve({
            status: res.statusCode,
            body: JSON.parse(Buffer.concat(chunks)),
          }),
        );
        res.on("error", reject);
      },
    );
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
      return await new Promise((resolve) => (finish = resolve));
    },
  });
  t.after(() => bridge.close());
  assert.equal((await lstat(dirname(bridge.socketPath))).mode & 0o777, 0o700);
  assert.equal((await lstat(bridge.socketPath)).mode & 0o777, 0o600);
  const call = { id: "toolu_once", tool: "Bash", input: { command: "effect" } };
  assert.equal((await post(bridge, "/tool", call, "wrong")).status, 403);
  assert.equal((await post(bridge, "/tool", { ...call, tool: "Agent" })).status, 400);
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

test("long sessions pass the former lifetime limit without permitting old effects to replay", async () => {
  let effects = 0;
  const bridge = await startModBridge({
    async nativeCall(_tool, input) {
      effects++;
      if (input.command === "lost") throw new Error("lost result after effect");
      return { result: { stdout: "ok" } };
    },
    async runHook() {
      effects++;
      return { exitCode: 0, stdout: "", stderr: "" };
    },
  });
  const call = (id) => ({ id, tool: "Bash", input: { command: "effect" } });
  try {
    await post(bridge, "/cancel", { id: "cancelled_before_admission" });
    await post(bridge, "/tool", { ...call("lost_result"), input: { command: "lost" } });
    for (let i = 0; i < 16400; i++) {
      const hook = i % 2 === 0;
      const result = await post(
        bridge,
        hook ? "/hook" : "/tool",
        hook ? { id: `long_${i}`, command: "guard", input: {}, timeout: 1 } : call(`long_${i}`),
      );
      assert.equal(result.status, 200, `request ${i}: ${JSON.stringify(result.body)}`);
      assert.ok(hook ? result.body.hook : result.body.result);
    }
    assert.equal((await post(bridge, "/tool", call("long_0"))).body.code, "duplicate_call");
    assert.equal((await post(bridge, "/tool", call("lost_result"))).body.code, "duplicate_call");
    assert.equal(
      (await post(bridge, "/tool", call("cancelled_before_admission"))).body.code,
      "call_cancelled",
    );
    assert.equal(effects, 16401);
    const files = (await readdir(dirname(bridge.socketPath))).filter((name) =>
      name.startsWith("spent-"),
    );
    assert.ok(files.length <= 256);
    assert.equal((await lstat(`${dirname(bridge.socketPath)}/${files[0]}`)).mode & 0o777, 0o600);
  } finally {
    await bridge.close();
  }
  await assert.rejects(lstat(dirname(bridge.socketPath)), { code: "ENOENT" });
});

test("spent calls spill exactly, keep bounded memory, and refuse missing storage", async () => {
  const directory = await mkdtemp("/tmp/cowboy-ledger-test-");
  try {
    const ledger = new SpentCalls(directory);
    for (let i = 0; i < 5000; i++) assert.equal(ledger.claim(`id_${i}`, "admitted"), undefined);
    assert.equal(ledger.cache.size, 4096);
    assert.equal(ledger.claim("id_0", "admitted"), "admitted");
    assert.equal(ledger.claim("cancelled", "cancelled"), undefined);
    assert.equal(ledger.claim("cancelled", "admitted"), "cancelled");
    ledger.cache.clear();
    await rm(ledger.path("id_0"));
    assert.throws(() => ledger.get("id_0"), { code: "bridge_storage_unavailable" });
    assert.throws(() => ledger.claim("fresh", "admitted"), { code: "bridge_storage_unavailable" });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("admission storage failure blocks new effects with a specific safe diagnostic", async () => {
  let effects = 0;
  const bridge = await startModBridge({
    async nativeCall() {
      effects++;
      return { result: {} };
    },
  });
  try {
    const first = { id: "storage_first", tool: "Bash", input: { command: "effect" } };
    await post(bridge, "/tool", first);
    const bucket = (id) => createHash("sha256").update(id).digest("hex").slice(0, 2);
    let counter = 0,
      second = "storage_next_0";
    while (bucket(second) !== bucket(first.id)) second = `storage_next_${++counter}`;
    await rm(`${dirname(bridge.socketPath)}/spent-${bucket(first.id)}`);
    const result = await post(bridge, "/tool", { ...first, id: second });
    assert.equal(result.status, 500);
    assert.equal(result.body.code, "bridge_storage_unavailable");
    assert.equal(effects, 1);
  } finally {
    await bridge.close();
  }
});

test("same-size ledger corruption cannot turn a spent identity into a fresh effect", async () => {
  const directory = await mkdtemp("/tmp/cowboy-ledger-corrupt-");
  try {
    const ledger = new SpentCalls(directory);
    ledger.claim("first", "admitted");
    ledger.cache.clear();
    await writeFile(ledger.path("first"), "\nother\tadmitted\n");
    assert.throws(() => ledger.claim("first", "admitted"), { code: "bridge_storage_unavailable" });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("the concurrent request limit is temporary and does not consume a refused identity", async () => {
  const pending = [];
  const bridge = await startModBridge(
    {
      nativeCall(_name, input) {
        return input.command === "ready"
          ? { result: {} }
          : new Promise((resolve) => pending.push(resolve));
      },
    },
    { waitMs: 1 },
  );
  const call = (id) => ({ id, tool: "Bash", input: { command: "hold" } });
  try {
    for (let i = 0; i < 32; i++)
      assert.equal((await post(bridge, "/tool", call(`held_${i}`))).status, 202);
    const busy = await post(bridge, "/tool", call("later"));
    assert.equal(busy.status, 409);
    assert.equal(busy.body.code, "bridge_busy");
    for (const resolve of pending) resolve({ result: {} });
    for (let i = 0; i < 32; i++)
      assert.equal((await post(bridge, "/result", { id: `held_${i}` })).status, 200);
    assert.equal(
      (await post(bridge, "/tool", { ...call("later"), input: { command: "ready" } })).status,
      200,
    );
  } finally {
    await bridge.close();
  }
});

test("request saturation and unknown failures keep diagnostics without exposing exception text", async () => {
  const bridge = await startModBridge({
    async runHook({ command }) {
      throw new Error(
        command === "busy" ? "Too many execution requests" : "secret token and project path",
      );
    },
  });
  try {
    for (const [command, code] of [
      ["busy", "execution_request_limit"],
      ["private", "execution_unavailable"],
    ]) {
      const result = await post(bridge, "/hook", { id: command, command, input: {}, timeout: 1 });
      assert.equal(result.body.code, code);
      assert.ok(!JSON.stringify(result).includes("secret token"));
    }
  } finally {
    await bridge.close();
  }
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
  const bridge = await startModBridge(
    {
      nativeCall() {
        effects++;
        return new Promise((resolve) => (finish = resolve));
      },
    },
    { waitMs: 5 },
  );
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
  assert.equal((await post(bridge, "/result", { id: call.id })).body.result.stdout, "done");
  assert.equal((await post(bridge, "/result", { id: call.id })).status, 409);
  assert.equal(effects, 1);
});
