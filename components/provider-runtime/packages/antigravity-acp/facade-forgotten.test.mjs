// Evidence rule E1 (docs/uncertain-effect-recovery.md): a process the
// executor forgot (30 s after it closed, as during a long Controller outage)
// ended. It must never be misread as an unconfirmed termination.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { Connection } from "../../execution-connection.mjs";
import { Facade } from "./facade.mjs";
import { buildProjection } from "./projection.mjs";
import { fakeEndpoint } from "./test-endpoint.mjs";

const hasRg = (() => {
  try {
    execFileSync("rg", ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
})();

// The executor forgets a closed process after `RETAIN` ms; a held read
// (`HOLD`, a worker waiting out a Controller restart) reaches it later.
const RETAIN = 100;
const HOLD = 600;

async function fixture(t, options = {}) {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-forgotten-")));
  const root = join(base, "workspace");
  mkdirSync(root, { recursive: true });
  writeFileSync(join(root, "hello.txt"), "hello target\n");
  const socket = join(base, "exec.sock");
  const endpoint = await fakeEndpoint(socket, root, { retainClosedMs: RETAIN, ...options });
  const connection = await Connection.open(
    {
      schema: 1,
      endpoint: `unix:${socket}`,
      bearer_token: "a".repeat(64),
      binding: { schema: 1, environment: { id: "env" }, workspace: { cwd: root } },
    },
    { clientName: "cowboy-antigravity-test", requestTimeoutMs: 20000 },
  );
  const uncertain = [];
  const facade = await Facade.create(connection, {
    onUncertain: (session, method) => uncertain.push([session, method]),
  });
  t.after(() => {
    connection.close();
    endpoint.close();
    rmSync(base, { recursive: true, force: true });
  });
  facade.beginTurn("s1");
  const call = async (name, args) => {
    const result = await facade.call("s1", name, args);
    return { text: result.content[0].text, error: result.isError === true };
  };
  const methods = () => endpoint.calls.map((entry) => entry.method);
  return { base, root, facade, call, endpoint, uncertain, methods };
}

/** Holds every `process/read` of a process started after `armed()`. */
function holdReads(times = 1) {
  const state = { armed: false, started: new Set(), held: 0 };
  return {
    arm: () => {
      state.armed = true;
    },
    delay: (method, params) => {
      if (!state.armed) return 0;
      if (method === "process/start") state.started.add(params.processId);
      if (method === "process/read" && state.started.has(params.processId) && state.held < times) {
        state.held += 1;
        return HOLD;
      }
      return 0;
    },
  };
}

test("an edit whose helper was forgotten before its reads arrived is verified, not uncertain", async (t) => {
  const hold = holdReads();
  const { root, call, uncertain, methods } = await fixture(t, { delay: hold.delay });
  await call("ReadFile", { path: "hello.txt" });
  hold.arm();
  const edited = await call("EditFile", {
    path: "hello.txt",
    old_string: "hello",
    new_string: "bye",
  });
  assert.equal(edited.error, false, edited.text);
  assert.match(
    edited.text,
    /^Edited .*hello\.txt: the requested content is verified on the target/,
  );
  assert.deepEqual(uncertain, []);
  assert.equal(readFileSync(join(root, "hello.txt"), "utf8"), "bye target\n");
  // Nothing was signalled: the helper had already ended.
  assert.ok(!methods().includes("process/terminate"));
  // The turn goes on, and the new bytes count as read.
  const again = await call("EditFile", { path: "hello.txt", old_string: "bye", new_string: "hi" });
  assert.equal(again.error, false, again.text);
});

test("a forgotten helper whose result is not on the target stays uncertain and needs a fresh read", async (t) => {
  const hold = holdReads();
  const { base, facade, call, uncertain } = await fixture(t, {
    delay: hold.delay,
    // A helper that ends without publishing or reporting.
    fileHelper: (() => {
      const path = join(tmpdir(), `agy-silent-helper-${process.pid}-${Date.now()}`);
      writeFileSync(path, "#!/bin/sh\nexit 0\n");
      chmodSync(path, 0o755);
      t.after(() => rmSync(path, { force: true }));
      return path;
    })(),
  });
  assert.ok(base);
  await call("ReadFile", { path: "hello.txt" });
  hold.arm();
  const edited = await call("EditFile", {
    path: "hello.txt",
    old_string: "hello",
    new_string: "bye",
  });
  assert.equal(edited.error, true);
  assert.match(edited.text, /^outcome unknown/);
  assert.deepEqual(uncertain, [["s1", "file publish"]]);
  await facade.endTurn("s1");
  facade.beginTurn("s1");
  const next = await call("EditFile", {
    path: "hello.txt",
    old_string: "hello",
    new_string: "bye",
  });
  assert.equal(next.error, true);
  assert.match(next.text, /Read hello\.txt with ReadFile before editing it/);
});

test("a command forgotten while its read was held reports an unknown exit, not an uncertain effect", async (t) => {
  const hold = holdReads();
  const { call, uncertain, methods } = await fixture(t, { delay: hold.delay });
  hold.arm();
  const ran = await call("RunCommand", { command: "printf done" });
  assert.equal(ran.error, false, ran.text);
  assert.match(ran.text, /^Exit code: unknown \(the command's process ended while the connection/);
  assert.deepEqual(uncertain, []);
  assert.ok(!methods().includes("process/terminate"));
});

test("a worker refusal or another id naming an unknown process is not evidence", async (t) => {
  for (const failure of [
    (id) => ({ code: -32000, message: `unknown process id ${id}` }),
    () => ({ code: -32600, message: "unknown process id cowboy-agy-other" }),
  ]) {
    let armed = false;
    const { call, uncertain } = await fixture(t, {
      fail: (method, params) =>
        armed && method === "process/read" ? failure(params.processId) : undefined,
    });
    await call("ReadFile", { path: "hello.txt" });
    armed = true;
    const edited = await call("EditFile", {
      path: "hello.txt",
      old_string: "hello",
      new_string: "bye",
    });
    assert.equal(edited.error, true);
    assert.equal(uncertain.length, 1, edited.text);
  }
});

test("a search whose process was forgotten answers a retryable error, never a partial result", {
  skip: !hasRg,
}, async (t) => {
  const hold = holdReads();
  const { call, uncertain } = await fixture(t, { delay: hold.delay });
  hold.arm();
  const found = await call("SearchFiles", { pattern: "hello" });
  assert.equal(found.error, true);
  assert.match(found.text, /output is unavailable; run it again/);
  assert.deepEqual(uncertain, []);
  const again = await call("SearchFiles", { pattern: "hello" });
  assert.match(again.text, /hello\.txt:1:hello target/);
});

test("a forgotten readiness probe reports the Git state unavailable and never runs again", async (t) => {
  const hold = holdReads();
  const { root, facade, endpoint } = await fixture(t, { delay: hold.delay });
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"]);
  await facade.endTurn("s1");
  hold.arm();
  const projection = await buildProjection(facade, root);
  assert.match(projection, /Git: state unavailable/);
  assert.doesNotMatch(projection, /Git status: clean|not a repository/);
  const starts = endpoint.calls.filter((entry) => entry.method === "process/start");
  assert.equal(starts.length, 1);
});

test("a stop refused before it was sent is sent again later", async (t) => {
  let refuse = true;
  const { facade } = await fixture(t);
  const turn = facade.session("s1").turn;
  const process = await facade.start(turn, ["/bin/sh", "-c", "sleep 30"]);
  const call = facade.connection.call.bind(facade.connection);
  facade.connection.call = (method, ...rest) =>
    method === "process/terminate" && refuse
      ? Promise.reject(new Error("Too many execution requests"))
      : call(method, ...rest);
  await assert.rejects(facade.terminate(process), /Too many execution requests/);
  refuse = false;
  assert.equal(await facade.terminate(process), true);
});

test("a lost reply to a command's deadline stop is an uncertain effect, and no second stop follows", async (t) => {
  const { facade, call, uncertain, methods } = await fixture(t, {
    lose: (method) => method === "process/terminate",
  });
  const ran = await call("RunCommand", { command: "sleep 5", timeout_seconds: 1 });
  assert.equal(ran.error, true);
  assert.match(ran.text, /^outcome unknown/);
  assert.deepEqual(uncertain, [["s1", "process/terminate"]]);
  // The turn's cleanup only observes the command it may already have stopped.
  await facade.endTurn("s1", { cancelled: true });
  assert.equal(methods().filter((method) => method === "process/terminate").length, 1);
});

test("a helper whose process tree exits slowly after its stop is confirmed, not uncertain", async (t) => {
  const { facade, uncertain } = await fixture(t);
  const turn = facade.session("s1").turn;
  // Exits 7 s after SIGTERM, beyond the former 5 s confirmation.
  const result = await facade.runBounded(
    turn,
    [
      "/bin/sh",
      "-c",
      'sleep 100 >/dev/null 2>&1 & child=$!; trap \'kill "$child"; wait "$child"; sleep 7; exit 0\' TERM; wait',
    ],
    { timeoutMs: 300, maxBytes: 1024 },
  );
  assert.equal(result.timedOut, true);
  assert.equal(turn.uncertain, undefined);
  assert.deepEqual(uncertain, []);
});

test("an ended turn waits for a slow-exiting command to stop instead of leaving it to expiry", async (t) => {
  const { facade, call, methods } = await fixture(t);
  // Exits 7 s after SIGTERM, beyond the former 5 s turn-end confirmation.
  const ran = await call("RunCommand", {
    command:
      'sleep 100 >/dev/null 2>&1 & child=$!; trap \'kill "$child"; wait "$child"; sleep 7; exit 0\' TERM; wait',
    // Long enough for the shell to install its trap.
    wait_seconds: 1,
  });
  assert.match(ran.text, /^Still running/);
  const started = Date.now();
  await facade.endTurn("s1");
  assert.ok(Date.now() - started >= 6000, "the turn end waited for the exit");
  assert.equal(facade.session("s1").commands.size, 0, "the stop was confirmed at turn end");
  assert.equal(methods().filter((method) => method === "process/terminate").length, 1);
});

test("the turn's end joins a stop still in flight instead of leaving its reply outstanding", async (t) => {
  let held = 0;
  const { facade, call, endpoint } = await fixture(t, {
    // The first stop's reply is held for a second.
    delay: (method) => (method === "process/terminate" && held++ === 0 ? 1000 : 0),
  });
  const ran = await call("RunCommand", { command: "sleep 30", wait_seconds: 1 });
  assert.match(ran.text, /^Still running \(command_id: (\S+)\)/);
  const id = ran.text.match(/command_id: (\S+)\)/)[1];
  // A StopCommand sends the stop; the turn ends while its reply is held.
  const stopping = call("StopCommand", { command_id: id });
  await new Promise((done) => setTimeout(done, 100));
  await facade.endTurn("s1");
  const answered = endpoint.calls.filter((entry) => entry.method === "process/terminate");
  assert.equal(answered.length, 1, "one stop, joined by the turn's end");
  // The stop's reply arrived before the turn's end returned.
  assert.ok(Date.now() - answered[0].at >= 1000);
  await stopping;
});

test("a command whose start is still pending when the turn ends is stopped by the turn's end", async (t) => {
  const { facade, call } = await fixture(t, {
    delay: (method) => (method === "process/start" ? 500 : 0),
  });
  const running = call("RunCommand", { command: "sleep 30", wait_seconds: 0 });
  await new Promise((done) => setTimeout(done, 100));
  await facade.endTurn("s1");
  assert.equal(facade.session("s1").commands.size, 0, "registered, then stopped and forgotten");
  await running;
});

test("at exit the connection waits for in-flight requests, bounded, and the facade starts nothing new", async (t) => {
  let hold = 0;
  const { facade, call, endpoint } = await fixture(t, {
    delay: (method) => (method === "process/read" && hold > 0 ? hold : 0),
  });
  const ran = await call("RunCommand", { command: "sleep 30", wait_seconds: 0 });
  assert.match(ran.text, /^Still running/);
  const connection = facade.connection;
  assert.equal(await connection.idle(10), true, "idle: immediate");
  hold = 600;
  const process = [...facade.session("s1").commands.values()][0];
  const read = facade.read(process, 0, { lostIsUncertain: false });
  // Sent once its connection check settled.
  await new Promise((done) => setTimeout(done, 50));
  assert.equal(await connection.idle(50), false, "a held reply bounds the wait");
  assert.equal(await connection.idle(5000), true, "then drains once answered");
  await read;
  // Quiesced: the command's expiry starts no stop or read.
  facade.quiesce();
  const before = endpoint.calls.length;
  await facade.expire(facade.session("s1"), process);
  assert.equal(endpoint.calls.length, before);
});
