import assert from "node:assert/strict";
import { request } from "node:http";
import test from "node:test";
import { DESCRIPTIONS } from "../plugins/claude-code/runtime/tools.mjs";
import { startModBridge } from "../plugins/claude-code/runtime/mod-bridge.mjs";
import {
  nativeArguments,
  PermissionBroker,
  permissionRequest,
  permissionResult,
  startingPermissionMode,
} from "../plugins/claude-code/runtime/launch.mjs";
import {
  insideWorkspace,
  permissionInput,
  targetPath,
} from "../plugins/claude-code/runtime/context-mod.js";

let fixtureId = 0;
async function modFixture(check) {
  const { register } = await import(
    `../plugins/claude-code/runtime/context-mod.js?permissions=${++fixtureId}`
  );
  const hooks = new Map();
  register((name, matcher, handler) => {
    if (!hooks.has(name)) hooks.set(name, { handler: handler ?? matcher });
    return { catch: () => {} };
  });
  const posts = [];
  const checks = [];
  const responses = new Map();
  const api = {
    env: { get: () => "/fixture/context.json" },
    fs: {
      read: () =>
        JSON.stringify({
          schema: 1,
          nonce: "a".repeat(32),
          bridgeToken: "b".repeat(64),
          socketPath: "/tmp/cowboy-claude-mod-fixture/bridge.sock",
          descriptions: DESCRIPTIONS,
          environment: "target environment",
          instructionFiles: [],
          git: "target git",
          agents: {},
          targetCwd: "/target",
          runtimeCwd: "/runtime",
          targetHome: "/home/target",
          hooks: { commands: [], tool: {} },
          skills: {
            prefix: "cowboy-target:",
            entries: [],
            omitted: [],
            bundled: [],
            unavailable: {},
          },
          memory: false,
        }),
    },
    command: { register: () => {} },
    tool: {
      check: async (value) => {
        checks.push(value);
        return await check(value);
      },
    },
    http: {
      fetch: async (url, options) => {
        const path = url.slice("http://cowboy-execution".length);
        posts.push({ path, body: JSON.parse(options.body) });
        return responses.get(path)?.() ??
          (path === "/link"
            ? { ok: true, status: 200, text: '{"symlink":false}' }
            : undefined) ??
          { ok: true, status: 200, text: '{"ready":true}' };
      },
    },
  };
  await hooks.get("session.start").handler(api, {}, (event) => event);
  posts.length = 0;
  const call = (event, next = () => assert.fail("native body ran")) =>
    hooks.get("tool.call").handler(api, event, next);
  return { call, posts, checks, responses };
}

const ok = (value) => () => ({
  ok: true,
  status: 200,
  text: JSON.stringify(value),
});

test("native permission decisions gate target calls before any effect", async (t) => {
  await t.test("allow runs the original input", async () => {
    const { call, posts, checks } = await modFixture(() => ({
      decision: "allow",
    }));
    await call({ tool: "Read", tool_use_id: "t1", file_path: "a.txt" });
    // Mapped, as written (target-path rules), then outside: still allowed,
    // so no target resolution is needed.
    assert.deepEqual(checks, [
      { tool: "Read", input: { file_path: "/runtime/a.txt" } },
      { tool: "Read", input: { file_path: "a.txt" } },
      {
        tool: "Read",
        input: { file_path: "/.cowboy-target-outside/target/a.txt" },
      },
    ]);
    assert.deepEqual(posts.map(({ path }) => path), ["/tool"]);
  });
  await t.test("deny never reaches the target", async () => {
    const { call, posts } = await modFixture(() => ({
      decision: "deny",
      reason: "Bash(rm *) is denied",
    }));
    const result = await call({
      tool: "Bash",
      tool_use_id: "t2",
      command: "rm -rf x",
    });
    assert.match(result.deny, /Bash\(rm \*\) is denied/);
    assert.deepEqual(posts, []);
  });
  await t.test("a failed check fails closed", async () => {
    const { call, posts } = await modFixture(() => {
      throw new Error("engine");
    });
    const result = await call({ tool: "Write", tool_use_id: "t3" });
    assert.match(result.deny, /Permission check unavailable/);
    assert.deepEqual(posts, []);
  });
  await t.test("approved asks run the host's amended input", async () => {
    const { call, posts, responses } = await modFixture(() => ({
      decision: "ask",
      reason: "needs approval",
    }));
    let polls = 0;
    responses.set(
      "/permission",
      () =>
        ++polls < 3 ? { ok: true, status: 202, text: '{"pending":"t4"}' } : ok({
          behavior: "allow",
          updatedInput: { command: "echo amended" },
        })(),
    );
    await call({
      tool: "Bash",
      tool_use_id: "t4",
      agentId: "child",
      command: "echo original",
    });
    assert.deepEqual(posts.map(({ path }) => path), [
      "/permission",
      "/permission",
      "/permission",
      "/tool",
    ]);
    assert.deepEqual(posts[0].body, {
      id: "t4",
      tool: "Bash",
      input: { command: "echo original" },
      reason: "needs approval",
      owner: "child",
    });
    assert.deepEqual(posts[3].body.input, { command: "echo amended" });
  });
  await t.test("a denied ask returns the host's message", async () => {
    const { call, posts, responses } = await modFixture(() => ({
      decision: "ask",
    }));
    responses.set(
      "/permission",
      ok({ behavior: "deny", message: "Not this file" }),
    );
    const result = await call({
      tool: "Edit",
      tool_use_id: "t5",
      file_path: "a",
    });
    assert.equal(result.deny, "Not this file");
    assert.deepEqual(posts.map(({ path }) => path), ["/permission"]);
  });
  await t.test("an abandoned approval is withdrawn and never runs", async () => {
    const controller = new AbortController();
    const { call, posts, responses } = await modFixture(() => ({
      decision: "ask",
    }));
    responses.set("/permission", () => {
      controller.abort();
      return { ok: true, status: 202, text: '{"pending":"t6"}' };
    });
    const result = await call(
      { tool: "Bash", tool_use_id: "t6", command: "x" },
      Object.assign(() => assert.fail("native body ran"), {
        signal: controller.signal,
      }),
    );
    assert.match(result.deny, /cancelled/);
    await new Promise((resolve) => setImmediate(resolve));
    assert.deepEqual(posts.map(({ path }) => path), ["/permission", "/cancel"]);
  });
});

test("permission paths are judged as target paths", () => {
  const paths = {
    targetCwd: "/target",
    runtimeCwd: "/runtime",
    targetHome: "/home/target",
  };
  const map = (value, outside = false) =>
    permissionInput({ file_path: value, other: "/target/x" }, paths, outside)
      .file_path;
  assert.equal(map("/target/a/b.txt"), "/runtime/a/b.txt");
  assert.equal(map("/target"), "/runtime");
  assert.equal(map("relative.txt"), "/runtime/relative.txt");
  assert.equal(map("./a/../b.txt"), "/runtime/b.txt");
  // Outside the target workspace, even where the runtime has its workspace.
  assert.equal(map("../escape.txt"), "/.cowboy-target-outside/escape.txt");
  assert.equal(
    map("/runtime/outside.txt"),
    "/.cowboy-target-outside/runtime/outside.txt",
  );
  assert.equal(
    map("/target-other/x"),
    "/.cowboy-target-outside/target-other/x",
  );
  assert.equal(map("~/notes"), "/.cowboy-target-outside/home/target/notes");
  assert.equal(map("/target/a", true), "/.cowboy-target-outside/target/a");
  assert.equal(
    permissionInput({ file_path: "~/x" }, { ...paths, targetHome: null })
      .file_path,
    "/.cowboy-target-outside/~/x",
  );
  assert.equal(
    permissionInput({ file_path: "/etc/x" }, { ...paths, targetCwd: "/" })
      .file_path,
    "/runtime/etc/x",
  );
  assert.equal(
    permissionInput({ notebook_path: "n.ipynb", path: "src" }, paths).path,
    "/runtime/src",
  );
  assert.equal(targetPath("~/a", paths), "/home/target/a");
  assert.ok(insideWorkspace("/target/a", "/target"));
  assert.ok(!insideWorkspace("/targetx", "/target"));
});

test("workspace-scoped write allowances follow the target's real path", async (t) => {
  // Inside allowed, outside asked: the acceptEdits shape.
  const scoped = ({ input }) => ({
    decision: input.file_path?.startsWith("/runtime") ? "allow" : "ask",
  });
  for (
    const [name, resolved, asked] of [
      ["real path inside", { path: "/target/real.txt" }, false],
      ["symlink outside", { path: "/etc/passwd" }, true],
      ["unresolved", { path: null }, true],
    ]
  ) {
    await t.test(name, async () => {
      const { call, posts, responses } = await modFixture(scoped);
      responses.set("/resolve", ok(resolved));
      responses.set("/permission", ok({ behavior: "deny", message: "no" }));
      await call({
        tool: "Edit",
        tool_use_id: "w",
        file_path: "link.txt",
        old_string: "a",
        new_string: "b",
      });
      assert.deepEqual(posts[0], {
        path: "/resolve",
        body: { path: "/target/link.txt" },
      });
      assert.equal(posts[1].path, asked ? "/permission" : "/tool");
      if (asked) assert.match(posts[1].body.reason, /link\.txt/);
    });
  }
  await t.test("Write onto a symlink is refused as natively", async () => {
    const { call, posts, responses } = await modFixture(scoped);
    responses.set("/link", ok({ symlink: true }));
    responses.set("/resolve", ok({ path: "/elsewhere/real.txt" }));
    const result = await call({
      tool: "Write",
      tool_use_id: "s",
      file_path: "link.txt",
      content: "x",
    });
    assert.match(
      result.deny,
      /Refusing to write \/target\/link\.txt: it is a symbolic link\. Write to the link's target path instead: \/elsewhere\/real\.txt/,
    );
    assert.deepEqual(posts.map(({ path }) => path), ["/link", "/resolve"]);
  });
  await t.test("an uninspectable Write target asks", async () => {
    const { call, posts, responses } = await modFixture(() => ({
      decision: "allow",
    }));
    responses.set("/link", ok({ symlink: null }));
    responses.set("/permission", ok({ behavior: "deny", message: "no" }));
    await call({
      tool: "Write",
      tool_use_id: "u",
      file_path: "a",
      content: "",
    });
    assert.deepEqual(posts.map(({ path }) => path), ["/link", "/permission"]);
  });
  await t.test("a rule naming the target path decides", async () => {
    const { call, posts } = await modFixture(({ input }) =>
      input.file_path === "/target/secret"
        ? { decision: "deny", reason: "Edit(/target/secret)", rule: "x" }
        : { decision: "allow" }
    );
    const result = await call({
      tool: "Edit",
      tool_use_id: "d",
      file_path: "/target/secret",
    });
    assert.match(result.deny, /Edit\(\/target\/secret\)/);
    assert.deepEqual(posts, []);
  });
  await t.test("no target IO when the outside form is allowed too", async () => {
    const { call, posts } = await modFixture(() => ({ decision: "allow" }));
    await call({ tool: "Edit", tool_use_id: "e", file_path: "a.txt" });
    assert.deepEqual(posts.map(({ path }) => path), ["/tool"]);
  });
  await t.test("reads resolve like writes; outside paths need none", async () => {
    const { call, posts, responses } = await modFixture(scoped);
    responses.set("/resolve", ok({ path: "/target/a.txt" }));
    await call({ tool: "Read", tool_use_id: "r", file_path: "a.txt" });
    responses.set("/permission", ok({ behavior: "deny", message: "no" }));
    await call({ tool: "Read", tool_use_id: "o", file_path: "/etc/hosts" });
    assert.deepEqual(posts.map(({ path }) => path), [
      "/resolve",
      "/tool",
      "/permission",
    ]);
  });
  await t.test("bypass keeps commands naming the runtime path", async () => {
    const { call, posts } = await modFixture(() => ({ decision: "allow" }));
    await call({ tool: "Bash", tool_use_id: "p", command: "ls /runtime" });
    assert.deepEqual(posts.map(({ path }) => path), ["/tool"]);
  });
  await t.test("a command allowed only inside the runtime workspace asks", async () => {
    const { call, posts, responses } = await modFixture(({ input }) => ({
      decision: input.command.includes("/.cowboy-target-outside")
        ? "ask"
        : "allow",
    }));
    responses.set("/permission", ok({ behavior: "deny", message: "no" }));
    await call({
      tool: "Bash",
      tool_use_id: "b",
      command: "mkdir /runtime/x",
    });
    assert.deepEqual(posts.map(({ path }) => path), ["/permission"]);
  });
});

function post(bridge, path, value) {
  return new Promise((resolve, reject) => {
    const req = request({
      socketPath: bridge.socketPath,
      path,
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

test("bridge observes one host approval per call and withdraws it on cancel", async (t) => {
  const requests = [];
  const cancelled = [];
  const answers = new Map();
  const bridge = await startModBridge({
    nativeCall: async () => ({ result: {} }),
    cancelCall: async () => [],
  }, {
    waitMs: 5,
    permissions: {
      request: (call) =>
        new Promise((resolve) => {
          requests.push(call);
          answers.set(call.id, resolve);
        }),
      cancel: (id) => {
        cancelled.push(id);
        answers.get(id)({ behavior: "deny", message: "cancelled" });
      },
    },
  });
  t.after(() => bridge.close());
  const body = {
    id: "p1",
    tool: "Bash",
    input: { command: "x" },
    reason: null,
  };
  assert.equal((await post(bridge, "/permission", body)).status, 202);
  assert.equal((await post(bridge, "/permission", body)).status, 202);
  assert.equal(requests.length, 1);
  answers.get("p1")({ behavior: "allow" });
  assert.deepEqual((await post(bridge, "/permission", body)).body, {
    behavior: "allow",
  });
  // A delivered approval is not requested again under the same identity.
  assert.equal((await post(bridge, "/permission", body)).status, 202);
  assert.equal(requests.length, 2);
  await post(bridge, "/cancel", { id: "p1" });
  assert.deepEqual(cancelled, ["p1"]);
  assert.match(
    (await post(bridge, "/permission", body)).body.message,
    /unavailable/,
  );
  for (
    const invalid of [
      { ...body, id: "bad", tool: "Agent" },
      { ...body, id: "bad", reason: 3 },
      { ...body, id: "bad", owner: "../x" },
    ]
  ) assert.equal((await post(bridge, "/permission", invalid)).status, 400);
  // A hook decision withdraws only the prompt; the call stays admissible.
  const hooked = { ...body, id: "p2" };
  assert.equal((await post(bridge, "/permission", hooked)).status, 202);
  await post(bridge, "/withdraw", { id: "p2" });
  assert.deepEqual(cancelled, ["p1", "p2"]);
  assert.deepEqual(
    (await post(bridge, "/tool", { id: "p2", tool: "Bash", input: {} })).body,
    { result: {} },
  );
});

test("bridge without an approval channel denies asks", async (t) => {
  const bridge = await startModBridge({}, { waitMs: 5 });
  t.after(() => bridge.close());
  const result = await post(bridge, "/permission", {
    id: "p2",
    tool: "Write",
    input: {},
    reason: null,
  });
  assert.equal(result.body.behavior, "deny");
});

test("the broker asks the host in native shape and honors dontAsk", async () => {
  const sent = [];
  const results = [];
  const broker = new PermissionBroker("default");
  broker.send = async (frame) => sent.push(frame);
  broker.onResult = (result) => results.push(result);
  const call = {
    id: "toolu_1",
    tool: "Bash",
    input: { command: "rm -f gone.txt" },
    reason: "needs approval",
    owner: "child",
  };
  const pending = broker.request(call);
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].request, {
    subtype: "can_use_tool",
    tool_name: "Bash",
    display_name: "Bash",
    input: { command: "rm -f gone.txt" },
    permission_suggestions: [],
    tool_use_id: "toolu_1",
    agent_id: "child",
    decision_reason: "needs approval",
    description: "rm -f gone.txt",
  });
  // Native's own requests pass through untouched.
  assert.equal(
    broker.respond({
      type: "control_response",
      response: { request_id: "native", subtype: "success" },
    }),
    false,
  );
  assert.equal(
    broker.respond({
      type: "control_response",
      response: {
        request_id: sent[0].request_id,
        subtype: "success",
        response: { behavior: "allow", updatedInput: call.input },
      },
    }),
    true,
  );
  assert.deepEqual(await pending, {
    behavior: "allow",
    updatedInput: call.input,
  });
  assert.equal(results.length, 1);
  const withdrawn = broker.request({ ...call, id: "toolu_2" });
  broker.cancel("toolu_2");
  assert.equal((await withdrawn).behavior, "deny");
  assert.deepEqual(sent.at(-1), {
    type: "control_cancel_request",
    request_id: sent[1].request_id,
  });
  broker.mode = "dontAsk";
  const before = sent.length;
  assert.match((await broker.request(call)).message, /does not ask/);
  assert.equal(sent.length, before);
  assert.equal(
    permissionResult({ subtype: "error", error: "host gone" }).message,
    "host gone",
  );
  assert.equal(
    permissionResult({ subtype: "success", response: { behavior: "maybe" } })
      .behavior,
    "deny",
  );
  assert.equal(
    permissionRequest({ id: "x", tool: "Write", input: { file_path: "f" } })
      .description,
    "f",
  );
});

test("the selected mode governs instead of a forced bypass", () => {
  assert.equal(
    startingPermissionMode(nativeArguments([], "/plugin")),
    "default",
  );
  assert.equal(
    startingPermissionMode(
      nativeArguments(["--permission-mode=acceptEdits"], "/plugin"),
    ),
    "acceptEdits",
  );
  assert.ok(!nativeArguments([], "/plugin").includes("bypassPermissions"));
});
