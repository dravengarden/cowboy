import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
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
function layout(t, { writablePolicy = false, managed = false } = {}) {
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
  // The worker's own directory on the runtime Machine, where it opens sessions.
  const runtimeCwd = join(base, "runtime", "session");
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
      binding: {
        schema: 1,
        runtime: { machine_id: "ovh", cwd: runtimeCwd },
        environment: { id: "env-1" },
        workspace: { cwd: virtual },
        ...(managed ? { managed: { parent_session_id: "parent-1", profile: "read_only_v1" } } : {}),
      },
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
  return { base, archive, home, target, virtual, runtimeCwd, credentials, socket, paths, tmp };
}

function session(t, setup, environment = {}, args = []) {
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
    args,
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
  // The worker opens the session at its runtime directory; agy works in the
  // target's workspace.
  const created = await call("session/new", { cwd: setup.runtimeCwd, mcpServers: [] });
  assert.equal(created.result.sessionId, "fake-session");
  assert.equal(created.result.seen.params.cwd, setup.virtual);
  assert.equal(created.result.seen.hookProfile, "ordinary");
  assert.deepEqual(created.result.seen.tools, [
    "ReadFile",
    "ListDirectory",
    "SearchFiles",
    "WriteFile",
    "EditFile",
    "RunCommand",
    "StopCommand",
    "UpdatePlan",
    "Subagent",
    "StopSubagent",
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
  // The facade's answer, which agy never reports, is shown on its call.
  const read = notifications.find(
    (message) =>
      message.params?.update?.toolCallId === "call-read" &&
      message.params.update.status === "completed",
  ).params.update;
  assert.equal(read.title, "Read hello.txt");
  assert.deepEqual(read.content, [
    { type: "content", content: { type: "text", text: "TARGET_SENTINEL\n" } },
  ]);
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
      COWBOY_HOOK_PROFILE: "ordinary",
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

const ANNOUNCED = { schema: 1, profile: "read_only_v1", roundPath: "/m/child/round.json" };
const ROUND = { callId: "call-1", inputRevision: "rev-1", outputSchema: null };

test("a managed child gets only the read tools, no extensions, and its hook policy", async (t) => {
  const setup = layout(t, { managed: true });
  mkdirSync(join(setup.target, ".agents/skills/deploy"), { recursive: true });
  writeFileSync(
    join(setup.target, ".agents/skills/deploy/SKILL.md"),
    "---\nname: deploy\ndescription: Ship\n---\n",
  );
  const endpoint = await fakeEndpoint(setup.socket, setup.target, {
    virtual: setup.virtual,
    managed: ANNOUNCED,
  });
  t.after(() => endpoint.close());
  const { call } = session(t, setup);
  await call("initialize", { protocolVersion: 1 });
  const created = await call("session/new", { cwd: setup.runtimeCwd, mcpServers: [] });
  assert.deepEqual(created.result.seen.tools, [
    "ReadFile",
    "ListDirectory",
    "SearchFiles",
    "RunCommand",
    "UpdatePlan",
  ]);
  assert.equal(created.result.seen.hookProfile, "read-only-v1");
  const append = created.result.seen.params._meta.antigravity.systemPrompt.append;
  assert.match(append, /read-only Cowboy managed call/);
  assert.doesNotMatch(append, /WriteFile|cowboy_extensions|<skills>/);
  // No extension or memory request reached the worker's endpoint.
  assert.equal(
    endpoint.calls.some((entry) => entry.method.startsWith("cowboy/")),
    false,
  );
  const bare = await call("session/prompt", {
    sessionId: "fake-session",
    prompt: [{ type: "text", text: "read hello.txt" }],
  });
  assert.match(bare.error.message, /requires its Cowboy managed-call round/);
  const structured = await call("session/prompt", {
    sessionId: "fake-session",
    prompt: [{ type: "text", text: "read hello.txt" }],
    _meta: { "cowboy.dev/managedCall": { ...ROUND, outputSchema: [] } },
  });
  assert.match(structured.error.message, /JSON Schema object/);
  const prompt = await call("session/prompt", {
    sessionId: "fake-session",
    prompt: [{ type: "text", text: "read hello.txt" }],
    _meta: { "cowboy.dev/managedCall": ROUND },
  });
  assert.equal(prompt.result.stopReason, "end_turn");
  assert.equal(prompt.result.seenMeta, null);
  const mode = await call("session/set_mode", { sessionId: "fake-session", modeId: "plan" });
  assert.match(mode.error.message, /not available in a Cowboy managed call/);
});

test("a managed binding, its target and the launch profile must agree", async (t) => {
  // The binding is managed, the target enforces nothing.
  const unenforced = layout(t, { managed: true });
  const plain = await fakeEndpoint(unenforced.socket, unenforced.target, {
    virtual: unenforced.virtual,
  });
  t.after(() => plain.close());
  const first = session(t, unenforced);
  assert.match(
    (await first.call("initialize", { protocolVersion: 1 })).error.message,
    /requires its read-only execution environment/,
  );
  // The target enforces a profile the binding does not name.
  const unbound = layout(t);
  const announcing = await fakeEndpoint(unbound.socket, unbound.target, {
    virtual: unbound.virtual,
    managed: ANNOUNCED,
  });
  t.after(() => announcing.close());
  const second = session(t, unbound);
  assert.match(
    (await second.call("initialize", { protocolVersion: 1 })).error.message,
    /requires its read-only execution environment/,
  );
  // A launch flag or binding that disagrees before connecting starts no
  // agy ("a descriptor that does not validate starts no agy").
  assert.equal(
    agyEnvironment({ HOME: "/h" }, "/rt", { managed: true }).COWBOY_HOOK_PROFILE,
    "read-only-v1",
  );
});

test("the launcher refuses every managed profile", () => {
  const launcher = join(here, "..", "antigravity-launch", "launch.sh");
  for (const profile of ["read-only-v1", "image-v1"]) {
    const result = spawnSync("/bin/sh", [launcher, "acp", `--cowboy-managed-profile=${profile}`], {
      encoding: "utf8",
    });
    assert.equal(result.status, 64, profile);
    assert.match(result.stderr, /account-sandbox runtime unit/);
  }
});

test("a descriptor that does not validate starts no agy", async (t) => {
  const cases = [
    ["missing", () => ({ CREDENTIALS_DIRECTORY: undefined }), /no execution descriptor/],
    [
      "malformed",
      (setup) => {
        writeFileSync(join(setup.credentials, "descriptor"), "{not json");
        return {};
      },
      /JSON/,
    ],
    [
      "not private",
      (setup) => {
        chmodSync(join(setup.credentials, "descriptor"), 0o644);
        return {};
      },
      /private regular file/,
    ],
  ];
  const rewrite = (change) => (setup) => {
    const path = join(setup.credentials, "descriptor");
    const descriptor = JSON.parse(readFileSync(path, "utf8"));
    change(descriptor.binding);
    writeFileSync(path, JSON.stringify(descriptor));
    return {};
  };
  cases.push(
    [
      "managed without its parent",
      rewrite((binding) => {
        delete binding.managed.parent_session_id;
      }),
      /requires its read-only execution environment/,
    ],
    [
      "managed with an empty parent",
      rewrite((binding) => {
        binding.managed.parent_session_id = "";
      }),
      /requires its read-only execution environment/,
    ],
    [
      "managed with another profile",
      rewrite((binding) => {
        binding.managed.profile = "image_v1";
      }),
      /requires its read-only execution environment/,
    ],
    [
      "the managed flag with an ordinary binding",
      rewrite((binding) => {
        delete binding.managed;
      }),
      /requires a managed execution binding/,
      ["--cowboy-managed-profile=read-only-v1"],
    ],
    [
      "an unsupported profile flag",
      () => ({}),
      /Unsupported Cowboy managed profile/,
      ["--cowboy-managed-profile=image-v1"],
    ],
  );
  for (const [name, prepare, expected, args = []] of cases) {
    const setup = layout(t, { managed: true });
    // An agy that records that it ran.
    writeFileSync(
      setup.paths.agy,
      `#!/bin/sh\ntouch ${JSON.stringify(join(setup.base, "agy-started"))}\nexec ${JSON.stringify(process.execPath)} ${JSON.stringify(join(here, "testdata/fake-agy.mjs"))} "$@"\n`,
      { mode: 0o755 },
    );
    const { call } = session(t, setup, prepare(setup), args);
    const refused = await call("initialize", { protocolVersion: 1 });
    assert.match(refused.error.message, /^cowboy_antigravity_unavailable: /, name);
    assert.match(refused.error.message, expected, name);
    const opened = await call("session/new", { cwd: setup.virtual, mcpServers: [] });
    assert.match(opened.error.message, expected, name);
    assert.equal(existsSync(join(setup.base, "agy-started")), false, name);
  }
});
