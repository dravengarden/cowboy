// Turn-barrier probe of an official `agy` executable, run before the
// Antigravity Plugin pins it. A turn's backend evidence settles on a barrier
// request only the proxy sends (`evidence.mjs` `BARRIER_METHOD`), so a new
// `agy` must still log every request it reads in the line the evidence
// matches, and must answer the barrier "Method not found" (no effect).
//
// The executable runs with the proxy's own arguments and closed environment,
// in a fresh HOME and unauthenticated; the recipe gives it no network. No
// account is involved and no model request can be made. One turn is settled
// with the production `AgyLog`: it must find its barrier.
//
// Usage (see the `agy-barrier-probe` recipe):
//   node --import ./tools/register-memory-client.mjs tools/agy-barrier-probe.mjs \
//     --agy PATH --receipt PATH
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { parseArgs } from "node:util";
import {
  AgyLog,
  BARRIER_METHOD,
  requestBarrierPattern,
} from "../components/provider-runtime/packages/antigravity-acp/evidence.mjs";
import {
  AGY_ARGUMENTS,
  agyEnvironment,
} from "../components/provider-runtime/packages/antigravity-acp/launch.mjs";

const REPLY_TIMEOUT_MS = 30_000;

const { values } = parseArgs({
  options: { agy: { type: "string" }, receipt: { type: "string" } },
});
if (!values.agy || !values.receipt) {
  console.error("usage: agy-barrier-probe.mjs --agy PATH --receipt PATH");
  process.exit(2);
}
const agy = resolve(values.agy);
const bytes = await readFile(agy);
const digest = (name) => createHash(name).update(bytes).digest("hex");
const base = await mkdtemp(join(process.env.TMPDIR ?? tmpdir(), "agy-barrier-probe-"));
const home = join(base, "home");
const scratch = join(base, "tmp");
await mkdir(home);
await mkdir(scratch);

const child = spawn(agy, AGY_ARGUMENTS, {
  cwd: scratch,
  // The archive's `bin` is the executable's directory.
  env: agyEnvironment({ HOME: home, TMPDIR: scratch }, dirname(dirname(agy))),
  stdio: ["pipe", "pipe", "inherit"],
});
child.stdin.on("error", () => {});
const exited = new Promise((done) => child.once("close", (code, signal) => done({ code, signal })));
const waiters = new Map();
createInterface({ input: child.stdout }).on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  const waiter = waiters.get(message?.id);
  if (waiter) {
    waiters.delete(message.id);
    waiter(message);
  }
});
const write = (message) => child.stdin.write(`${JSON.stringify(message)}\n`);
const reply = (id, what) =>
  new Promise((done, fail) => {
    const timer = setTimeout(() => {
      waiters.delete(id);
      fail(new Error(`${what} was not answered`));
    }, REPLY_TIMEOUT_MS);
    waiters.set(id, (message) => {
      clearTimeout(timer);
      done(message);
    });
  });

const checks = [];
const check = (name, ok, detail = undefined) => {
  checks.push({ name, ok: Boolean(ok), ...(detail === undefined ? {} : { detail }) });
};
let receipt;
try {
  const initialized = reply(1, "initialize");
  write({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
    },
  });
  const init = await initialized;
  check("initialize answered", init.result?.protocolVersion === 1);

  let barrier;
  const log = new AgyLog(join(home, ".gemini", "antigravity-cli", "log"), child.pid, {
    request: (message) => {
      barrier = { method: message.method, reply: reply(message.id, message.method) };
      write(message);
    },
  });
  const cursor = await log.mark();
  check("the process log is found by pid", cursor !== undefined);
  const settled = await log.settle(cursor, "agy-barrier-probe", "end_turn");
  check("the turn settled on its barrier request", settled.barrier === true, settled);
  const answered = barrier ? await barrier.reply : undefined;
  check(
    "the barrier request has no effect (Method not found)",
    answered?.error?.code === -32601,
    answered?.error ?? answered?.result ?? null,
  );
  const nonce = barrier?.method.slice(`${BARRIER_METHOD}/`.length);
  const lines = (await readFile(await log.locate(), "utf8")).split("\n");
  const line = nonce && lines.find((text) => requestBarrierPattern(nonce).test(text));
  receipt = {
    schema: "cowboy.agy-barrier-probe.v1",
    agy: { sha256: digest("sha256"), sha512: digest("sha512"), bytes: bytes.length },
    arguments: AGY_ARGUMENTS,
    agent_info: init.result?.agentInfo ?? null,
    barrier_method: BARRIER_METHOD,
    // The glog prefix (time, thread) is the run's own; the rest is matched.
    barrier_line: line ? line.replace(/^[IWEF]\d{4} [\d:.]+\s+\d+ /, "") : null,
    checks,
  };
} catch (error) {
  check("probe ran", false, error.message);
  receipt = { schema: "cowboy.agy-barrier-probe.v1", checks };
} finally {
  child.stdin.end();
  const timer = setTimeout(() => child.kill("SIGKILL"), 5000);
  child.kill("SIGTERM");
  await exited;
  clearTimeout(timer);
  await rm(base, { recursive: true, force: true });
}
const passed = checks.every((item) => item.ok);
receipt.passed = passed;
await writeFile(values.receipt, `${JSON.stringify(receipt, null, 2)}\n`);
for (const item of checks) console.log(`${item.ok ? "ok  " : "FAIL"} ${item.name}`);
console.log(passed ? "agy barrier probe passed" : "agy barrier probe FAILED");
process.exit(passed ? 0 : 1);
