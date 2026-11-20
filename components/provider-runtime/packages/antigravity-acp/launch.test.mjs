import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  AGY_ARGUMENTS,
  agyEnvironment,
  checkMirror,
  checkPolicy,
  readDescriptor,
  runProxy,
} from "./launch.mjs";
import { fakeEndpoint } from "./test-endpoint.mjs";

const here = dirname(fileURLToPath(import.meta.url));

/** A runtime archive layout, an account home with read-only policy and a target. */
function layout(t, { writablePolicy = false } = {}) {
  // Unix socket paths must stay short; the nix shell TMPDIR is long.
  const base = realpathSync(mkdtempSync("/tmp/agy-proxy-"));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const archive = join(base, "runtime");
  for (const directory of ["bin", "policy", "proxy/app"]) {
    mkdirSync(join(archive, directory), { recursive: true });
  }
  copyFileSync(join(here, "hooks.json"), join(archive, "policy/hooks.json"));
  copyFileSync(join(here, "settings.json"), join(archive, "policy/settings.json"));
  writeFileSync(join(archive, "bin/agy-hook"), "#!/bin/sh\n", { mode: 0o755 });
  writeFileSync(
    join(archive, "bin/agy"),
    `#!/bin/sh\nexec ${JSON.stringify(process.execPath)} ${JSON.stringify(join(here, "testdata/fake-agy.mjs"))} "$@"\n`,
    { mode: 0o755 },
  );
  const home = join(base, "home");
  mkdirSync(join(home, ".gemini/config"), { recursive: true });
  mkdirSync(join(home, ".gemini/antigravity-cli"), { recursive: true });
  for (const [from, to] of [
    ["hooks.json", ".gemini/config/hooks.json"],
    ["settings.json", ".gemini/antigravity-cli/settings.json"],
  ]) {
    copyFileSync(join(here, from), join(home, to));
    if (!writablePolicy) chmodSync(join(home, to), 0o444);
  }
  const target = join(base, "target");
  // The session path the worker names; on the runtime side it does not exist.
  const virtual = join(base, "remote", "target");
  mkdirSync(target);
  writeFileSync(join(target, "hello.txt"), "TARGET_SENTINEL\n");
  const credentials = join(base, "credentials");
  mkdirSync(credentials, { mode: 0o700 });
  const socket = join(base, "exec.sock");
  writeFileSync(
    join(credentials, "descriptor"),
    JSON.stringify({
      schema: 1,
      endpoint: `unix:${socket}`,
      bearer_token: "b".repeat(64),
      binding: { schema: 1, environment: { id: "env-1" }, workspace: { cwd: virtual } },
    }),
    { mode: 0o600 },
  );
  const paths = {
    archive,
    agy: join(archive, "bin/agy"),
    hook: join(archive, "bin/agy-hook"),
    hooks: join(archive, "policy/hooks.json"),
    settings: join(archive, "policy/settings.json"),
    relay: join(here, "facade-relay.mjs"),
  };
  const tmp = join(base, "tmp");
  mkdirSync(tmp);
  return { base, archive, home, target, virtual, credentials, socket, paths, tmp };
}

function session(t, setup, environment = {}) {
  const input = new PassThrough();
  const output = new PassThrough();
  const replies = new Map();
  const notifications = [];
  let buffered = "";
  output.on("data", (chunk) => {
    buffered += chunk;
    let newline;
    while ((newline = buffered.indexOf("\n")) >= 0) {
      const message = JSON.parse(buffered.slice(0, newline));
      buffered = buffered.slice(newline + 1);
      if (message.id !== undefined) replies.get(message.id)?.(message);
      else notifications.push(message);
    }
  });
  const exited = runProxy({
    paths: setup.paths,
    environment: {
      HOME: setup.home,
      TMPDIR: setup.tmp,
      CREDENTIALS_DIRECTORY: setup.credentials,
      ...environment,
    },
    input,
    output,
  });
  t.after(async () => {
    input.end();
    await exited;
  });
  let next = 0;
  const call = (method, params) =>
    new Promise((resolve, reject) => {
      const id = ++next;
      const timer = setTimeout(() => reject(new Error(`${method} timed out`)), 20000);
      replies.set(id, (message) => {
        clearTimeout(timer);
        resolve(message);
      });
      input.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  return { call, notifications };
}

test("the proxy relays ACP through the facade over one execution connection", async (t) => {
  const setup = layout(t);
  const endpoint = await fakeEndpoint(setup.socket, setup.target, {
    virtual: setup.virtual,
  });
  t.after(() => endpoint.close());
  const { call, notifications } = session(t, setup);
  const initialized = await call("initialize", {
    protocolVersion: 1,
    clientCapabilities: { fs: { readTextFile: true }, terminal: true },
  });
  assert.deepEqual(initialized.result.seen.clientCapabilities, {
    fs: { readTextFile: false, writeTextFile: false },
    terminal: false,
  });
  const created = await call("session/new", { cwd: setup.virtual, mcpServers: [] });
  assert.equal(created.result.sessionId, "fake-session");
  assert.deepEqual(created.result.seen.tools, [
    "ReadFile",
    "ListDirectory",
    "SearchFiles",
    "WriteFile",
    "EditFile",
    "RunCommand",
    "StopCommand",
  ]);
  const server = created.result.seen.params.mcpServers;
  assert.equal(server.length, 1);
  assert.equal(server[0].name, "cowboy");
  assert.equal(server[0].command, process.execPath);
  assert.match(
    created.result.seen.params._meta.antigravity.systemPrompt.append,
    /<cowboy_routing>/,
  );
  const prompt = await call("session/prompt", {
    sessionId: "fake-session",
    prompt: [{ type: "text", text: "read hello.txt" }],
  });
  assert.equal(prompt.result.stopReason, "end_turn");
  assert.equal(prompt.result._meta.cowboy.antigravity_backend.class, "ok");
  assert.equal(notifications.at(-1).params.update.content.text, "TARGET_SENTINEL\n");
  const refused = await call("authenticate", { methodId: "oauth-personal" });
  assert.equal(refused.error.code, -32601);
  assert.equal(endpoint.calls.filter((entry) => entry.method === "initialize").length, 1);
});

test("a backend refusal in the log turns the prompt into a fixed error", async (t) => {
  const setup = layout(t);
  const endpoint = await fakeEndpoint(setup.socket, setup.target, {
    virtual: setup.virtual,
  });
  t.after(() => endpoint.close());
  writeFileSync(join(setup.home, "refuse"), "");
  const { call } = session(t, setup);
  await call("initialize", { protocolVersion: 1 });
  await call("session/new", { cwd: setup.virtual, mcpServers: [] });
  const prompt = await call("session/prompt", { sessionId: "fake-session", prompt: [] });
  assert.equal(prompt.error.message, "antigravity_backend_refusal:rate_limited");
});

test("readiness refuses initialize without its preconditions", async (t) => {
  const writable = layout(t, { writablePolicy: true });
  const endpoint = await fakeEndpoint(writable.socket, writable.target, {
    virtual: writable.virtual,
  });
  t.after(() => endpoint.close());
  const first = session(t, writable);
  const refused = await first.call("initialize", { protocolVersion: 1 });
  assert.match(
    refused.error.message,
    /^cowboy_antigravity_unavailable: policy file .* is writable/,
  );
  const unreachable = layout(t);
  const second = session(t, unreachable);
  assert.match(
    (await second.call("initialize", { protocolVersion: 1 })).error.message,
    /^cowboy_antigravity_unavailable: Execution connection/,
  );
  const missing = layout(t);
  const third = session(t, missing, { CREDENTIALS_DIRECTORY: undefined });
  assert.match(
    (await third.call("initialize", { protocolVersion: 1 })).error.message,
    /no execution descriptor/,
  );
});

test("descriptor, policy, mirror and environment checks", async (t) => {
  const setup = layout(t);
  const descriptor = await readDescriptor(setup.credentials);
  assert.equal(descriptor.endpoint, `unix:${setup.socket}`);
  chmodSync(join(setup.credentials, "descriptor"), 0o644);
  await assert.rejects(readDescriptor(setup.credentials), /private regular file/);
  chmodSync(join(setup.credentials, "descriptor"), 0o460);
  await assert.rejects(readDescriptor(setup.credentials), /private regular file/);
  // systemd's LoadCredential mode.
  chmodSync(join(setup.credentials, "descriptor"), 0o440);
  assert.equal((await readDescriptor(setup.credentials)).endpoint, `unix:${setup.socket}`);
  chmodSync(join(setup.credentials, "descriptor"), 0o600);
  writeFileSync(
    join(setup.credentials, "descriptor"),
    JSON.stringify({ ...descriptor, endpoint: "ws://127.0.0.1:1/" }),
  );
  await assert.rejects(readDescriptor(setup.credentials), /unsupported execution descriptor/);
  await checkPolicy(setup.home, setup.paths);
  // A hooks.json agy cannot parse would silently disable the gate.
  for (const path of [setup.paths.hooks, join(setup.home, ".gemini/config/hooks.json")]) {
    chmodSync(path, 0o644);
    writeFileSync(
      path,
      '{"cowboy-facade-gate":{"PreToolUse":[{"matcher":"read_file","hooks":[]}]}}',
    );
    chmodSync(path, 0o444);
  }
  await assert.rejects(checkPolicy(setup.home, setup.paths), /not Cowboy's/);
  chmodSync(setup.paths.hooks, 0o644);
  copyFileSync(join(here, "hooks.json"), setup.paths.hooks);
  chmodSync(join(setup.home, ".gemini/config/hooks.json"), 0o644);
  writeFileSync(join(setup.home, ".gemini/config/hooks.json"), "{}");
  chmodSync(join(setup.home, ".gemini/config/hooks.json"), 0o444);
  await assert.rejects(checkPolicy(setup.home, setup.paths), /differs/);
  rmSync(join(setup.home, ".gemini/config/hooks.json"));
  await assert.rejects(checkPolicy(setup.home, setup.paths), /missing/);
  assert.doesNotThrow(() => checkMirror(join(setup.base, "absent")));
  assert.doesNotThrow(() => checkMirror(setup.tmp));
  assert.throws(() => checkMirror(setup.target), /not empty/);
  assert.throws(() => checkMirror("relative"), /absolute/);
  assert.deepEqual(
    agyEnvironment(
      {
        HOME: "/h",
        TMPDIR: "/t",
        CREDENTIALS_DIRECTORY: "/c",
        XDG_RUNTIME_DIR: "/x",
        GEMINI_API_KEY: "k",
      },
      "/rt",
    ),
    {
      HOME: "/h",
      PATH: "/rt/bin:/usr/bin:/bin",
      LANG: "C.UTF-8",
      TZ: "UTC",
      TMPDIR: "/t",
      AGY_CLI_DISABLE_AUTO_UPDATE: "true",
      DBUS_SESSION_BUS_ADDRESS: "unix:path=/nonexistent",
    },
  );
  assert.deepEqual(AGY_ARGUMENTS, [
    "acp",
    "--disable-telemetry",
    "--include-default-customizations=false",
  ]);
});

test("the relay refuses to start without a valid channel", async () => {
  const child = spawn(process.execPath, [join(here, "facade-relay.mjs"), "/nonexistent", "bad"]);
  const [code] = await new Promise((done) => child.once("exit", (...result) => done(result)));
  assert.equal(code, 2);
});
