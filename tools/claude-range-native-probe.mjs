// Credential-free real pinned executor acceptance. No Controller or model call.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { WorkspaceTools } from "../plugins/claude-code/runtime/tools.mjs";

const [binary, receipt, helper] = process.argv.slice(2);
assert.ok(binary?.startsWith("/") && receipt?.startsWith("/"));
assert.ok(helper?.startsWith("/"));
const lock = JSON.parse(
  await readFile("components/execution-runtime/lock.json", "utf8"),
);
assert.equal(
  createHash("sha256").update(await readFile(binary)).digest("hex"),
  lock.executable_sha256,
);
const root = await mkdtemp(join(tmpdir(), "cowboy-range-native-"));
const target = join(root, "target");
const runtime = join(root, "runtime");
await mkdir(target);
await mkdir(runtime, { mode: 0o700 });
const child = spawn(binary, ["exec-server", "--listen", "stdio"], {
  cwd: target,
  env: {
    PATH: process.env.PATH,
    HOME: root,
    CODEX_HOME: join(root, "executor-home"),
  },
  stdio: ["pipe", "pipe", "ignore"],
});
let next = 0;
const pending = new Map();
const calls = [];
const lines = createInterface({ input: child.stdout });
lines.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.id === undefined) return;
  const request = pending.get(message.id);
  assert.ok(request);
  pending.delete(message.id);
  clearTimeout(request.timer);
  request.record.response_bytes = Buffer.byteLength(line);
  if (message.error) {
    const error = new Error("Native executor refused request");
    error.remote = message.error;
    request.reject(error);
  } else request.resolve(message.result);
});
child.on("exit", () => {
  for (const request of pending.values()) {
    clearTimeout(request.timer);
    request.reject(new Error("Native executor exited"));
  }
  pending.clear();
});
const connection = {
  async call(method, params) {
    const id = ++next;
    const frame = JSON.stringify({ id, method, params });
    const record = {
      method,
      request_bytes: Buffer.byteLength(frame),
      response_bytes: 0,
    };
    calls.push(record);
    return await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error("Native request timed out; no replay"));
        child.kill();
      }, 20000);
      pending.set(id, { resolve, reject, timer, record });
      child.stdin.write(frame + "\n");
    });
  },
};
try {
  await connection.call("initialize", {
    clientName: "cowboy-range-native-probe",
  });
  child.stdin.write(
    JSON.stringify({ method: "initialized", params: {} }) + "\n",
  );
  const binding = {
    workspace: { cwd: target },
    environment: { id: "fixture" },
  };
  const tools = new WorkspaceTools(
    connection,
    binding,
    join(runtime, "state.json"),
  );
  await tools.load();
  const original = ("x".repeat(127) + "\n").repeat(8192);
  const path = join(target, "file");
  await writeFile(path, original, { mode: 0o640 });
  await chmod(path, 0o640);
  // A target project cannot inject Python code into a native file Read.
  await writeFile(
    join(target, "json.py"),
    "raise RuntimeError('project module must not load')\n",
  );
  const measurements = [];
  for (const mode of ["whole_file", "target_range"]) {
    tools.fileHelper = mode === "target_range" ? helper : undefined;
    const before = calls.length;
    const start = performance.now();
    const result = await tools.nativeCall("Read", {
      file_path: "file",
      offset: 101,
      limit: 10,
    });
    assert.equal(result.deny, undefined);
    assert.equal(result.result.file.numLines, 10);
    assert.equal(
      result.result.file.content,
      Array(10).fill("x".repeat(127)).join("\n"),
    );
    const records = calls.slice(before);
    measurements.push({
      mode,
      elapsed_ms: performance.now() - start,
      rpc_count: records.length,
      methods: records.map((record) => record.method),
      rpc_json_bytes: records.reduce(
        (sum, record) => sum + record.request_bytes + record.response_bytes,
        0,
      ),
    });
  }
  assert.ok(
    measurements[1].rpc_json_bytes < measurements[0].rpc_json_bytes / 100,
  );
  await writeFile(path, original + "external change outside range");
  const conflict = await tools.nativeCall("Write", {
    file_path: "file",
    content: "lost",
  });
  assert.match(conflict.deny, /modified since read/);
  assert.equal(
    await readFile(path, "utf8"),
    original + "external change outside range",
  );
  const reread = await tools.nativeCall("Read", {
    file_path: "file",
    limit: 1,
  });
  assert.equal(reread.deny, undefined);
  const written = await tools.nativeCall("Write", {
    file_path: "file",
    content: "accepted\n",
  });
  assert.equal(written.deny, undefined);
  assert.equal(await readFile(path, "utf8"), "accepted\n");
  assert.deepEqual(tools.state.jobs, {});
  assert.equal(
    (await readFile(tools.statePath, "utf8")).includes("accepted"),
    false,
  );
  const quietStart = calls.length;
  const quiet = await tools.command(["bash", "-c", "sleep 2; printf quiet"]);
  assert.equal(quiet.output, "quiet");
  assert.equal(quiet.exitCode, 0);
  const quietReads = calls.slice(quietStart).filter((call) =>
    call.method === "process/read"
  ).length;
  const job = await tools.start(["bash", "-c", "sleep 60"]);
  const collecting = tools.collect(job, 120000);
  await new Promise((resolve) => setTimeout(resolve, 50));
  const cancelStart = performance.now();
  const cancelled = await tools.nativeCall("TaskStop", {
    task_id: job,
    timeout: 5000,
  });
  assert.equal(cancelled.deny, undefined);
  await collecting;
  const cancelMs = performance.now() - cancelStart;
  assert.ok(
    cancelMs < 5000,
    `Cancellation took ${cancelMs.toFixed(2)} ms; fixed budget is 5000 ms`,
  );
  assert.equal(tools.state.jobs[job].closed, true);
  const result = {
    schema_version: 1,
    acceptance: "pinned_native_executor_and_adapter_not_production_wan",
    native_version: lock.version,
    native_sha256: lock.executable_sha256,
    production_mutations: false,
    provider_inference_requested: false,
    measurements,
    quiet_command_process_reads: quietReads,
    cancel_while_collecting_ms: cancelMs,
    checks: [
      "identical_selected_bytes",
      "external_change_outside_range_refused",
      "reread_then_write_persists_on_target",
      "no_source_copy_in_runtime_state",
      "closed_private_jobs_reaped",
      "quiet_output_completed",
      "taskstop_wakes_pending_read_and_retains_handle",
    ],
  };
  await writeFile(resolve(receipt), JSON.stringify(result, null, 2) + "\n", {
    flag: "wx",
    mode: 0o600,
  });
  console.log("Pinned native range-read acceptance passed");
} finally {
  lines.close();
  if (child.exitCode === null && child.signalCode === null) {
    const exit = once(child, "exit");
    child.stdin.end();
    child.kill();
    const kill = setTimeout(() => child.kill("SIGKILL"), 5000);
    try {
      await exit;
    } finally {
      clearTimeout(kill);
    }
  }
  await rm(root, { recursive: true, force: true });
}
