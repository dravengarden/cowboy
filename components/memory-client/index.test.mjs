import assert from "node:assert/strict";
import {
  chmod,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  MatrixClient,
  matrixConfiguration,
  memoryConfiguration,
  memoryToolNames,
  sessionExtensions,
} from "./index.mjs";

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), "matrix-client-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [];
  let available = true;
  let contextAvailable = true;
  let observeWait = null;
  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const value = JSON.parse(Buffer.concat(chunks));
    requests.push({ path: request.url, ...value });
    if (request.url === "/v1/observe" && observeWait) await observeWait;
    if (
      request.headers.authorization !== "Bearer " + "a".repeat(64) ||
      !available ||
      request.url === "/v1/context" && !contextAvailable
    ) {
      response.writeHead(503).end("{}");
      return;
    }
    const result = request.url === "/v1/context"
      ? { text: "[Matrix memory]\nCurrent revision only." }
      : { events: ["observation:e0"] };
    response.writeHead(200, { "Content-Type": "application/json" }).end(
      JSON.stringify(result),
    );
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() =>
    new Promise((resolve) => {
      server.closeAllConnections();
      server.close(resolve);
    })
  );
  const config = {
    schema: 1,
    provider: "codex",
    endpoint: "http://127.0.0.1:" + server.address().port,
    token: "a".repeat(64),
    state_dir: join(root, "queue"),
    projects: [
      { workspace: "project-123", machine: "hawk", project: "cowboy" },
      {
        workspace: "different-project-id",
        machine: "falcon",
        project: "cowboy",
      },
      { path: "/local/cowboy", machine: "local", project: "cowboy" },
    ],
  };
  const descriptor = {
    binding: {
      id: "session-123",
      workspace: { id: "project-123", cwd: "/task/worktree-7" },
      environment: { machine_id: "hawk" },
    },
  };
  return {
    root,
    requests,
    config,
    descriptor,
    setAvailable: (value) => available = value,
    setContextAvailable: (value) => contextAvailable = value,
    setObserveWait: (value) => observeWait = value,
  };
}

test("host configuration refuses wrong Provider, insecure transport and symlinks", async (t) => {
  const f = await fixture(t);
  const path = join(f.root, "config.json");
  await writeFile(path, JSON.stringify(f.config), { mode: 0o600 });
  assert.equal(
    (await matrixConfiguration("codex", { path })).provider,
    "codex",
  );
  await assert.rejects(matrixConfiguration("claude", { path }), /Invalid/);
  const link = join(f.root, "link");
  await symlink(path, link);
  await assert.rejects(
    matrixConfiguration("codex", { path: link }),
    /private regular/,
  );
  await chmod(path, 0o644);
  await assert.rejects(
    matrixConfiguration("codex", { path }),
    /private regular/,
  );
  await chmod(path, 0o600);
  await writeFile(
    path,
    JSON.stringify({ ...f.config, endpoint: "http://remote.invalid" }),
  );
  await assert.rejects(matrixConfiguration("codex", { path }), /Invalid/);
  assert.equal(
    await matrixConfiguration("codex", { path: join(f.root, "absent") }),
    null,
  );
});

test("logical project identity spans Machines and ignores task directory names", async (t) => {
  const f = await fixture(t);
  const first = await MatrixClient.open(f.config, f.descriptor);
  const second = await MatrixClient.open(f.config, {
    binding: {
      id: "other-session",
      workspace: { id: "different-project-id" },
      environment: { machine_id: "falcon" },
    },
  });
  assert.deepEqual(first.binding, {
    project: "cowboy",
    machine: "hawk",
    session: "session-123",
  });
  assert.equal(second.binding.project, first.binding.project);
  assert.notEqual(second.binding.machine, first.binding.machine);
  // Without a usable mapping the session simply has no memory.
  assert.equal(
    await MatrixClient.open(f.config, undefined, { cwd: "/other/cowboy" }),
    null,
  );
  // A new Machine or workspace needs no enrollment; the service checks grants.
  const placed = (workspace, machine) => ({
    binding: {
      id: "s",
      workspace: { id: workspace },
      environment: { machine_id: machine },
    },
  });
  const newMachine = await MatrixClient.open(
    f.config,
    placed("project-123", "ovh"),
  );
  assert.equal(newMachine.binding.project, "cowboy");
  assert.equal(newMachine.binding.machine, "ovh");
  const newWorkspace = await MatrixClient.open(
    f.config,
    placed("fresh", "hawk"),
  );
  assert.equal(newWorkspace.binding.project, "fresh");
  await newMachine.close();
  await newWorkspace.close();
  await first.close();
  await second.close();
  await first.delivery;
  await second.delivery;
});

test("outage retains completed turns for restart delivery without stale recall", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(f.config, f.descriptor);
  assert.match(
    await client.begin("Use the release receipt."),
    /Current revision/,
  );
  f.setAvailable(false);
  assert.match(
    await client.begin(
      "Now forget the receipt. bearer abcdefghijklmnopqrstuvwxyz",
    ),
    /unavailable/,
  );
  assert.doesNotMatch(await client.begin("again"), /Current revision/);
  client.add("tool", "Verification passed.");
  client.add("assistant", "Recorded the result.");
  await client.close();
  await client.delivery;
  const names = (await readdir(client.directory)).filter((name) =>
    name.endsWith(".json")
  );
  assert.ok(names.length >= 2);
  for (const name of names) {
    assert.doesNotMatch(
      await readFile(join(client.directory, name), "utf8"),
      /abcdefghijklmnopqrstuvwxyz/,
    );
  }
  f.setAvailable(true);
  const restarted = await MatrixClient.open(f.config, f.descriptor);
  await restarted.flush();
  assert.equal(
    (await readdir(client.directory)).filter((name) => name.endsWith(".json"))
      .length,
    0,
  );
  assert.ok(
    f.requests.some((request) =>
      request.payload.events?.some((event) => event.role === "tool")
    ),
  );
  assert.ok(f.requests.some((request) => request.payload.learn === false));
  await restarted.close();
  await restarted.delivery;
});

test("current evidence is delivered once despite concurrent flush, then retired on receipt", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(f.config, f.descriptor);
  let release;
  f.setObserveWait(new Promise((resolve) => release = resolve));
  const begin = client.begin("Verify the exact release.");
  try {
    for (let i = 0; i < 100 && !f.requests.length; i++) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    assert.equal(f.requests.length, 1);
    await client.flush();
    assert.equal(
      f.requests.length,
      1,
      "flush must not duplicate reserved evidence",
    );
    assert.equal(
      (await readdir(client.directory)).filter((n) => n.endsWith(".json"))
        .length,
      1,
    );
  } finally {
    release();
  }
  assert.match(await begin, /Current revision/);
  await client.delivery;
  assert.equal(f.requests.filter((r) => r.path === "/v1/observe").length, 1);
  assert.equal(
    (await readdir(client.directory)).filter((n) => n.endsWith(".json")).length,
    0,
  );
  assert.equal(client.inFlight.size, 0);
  const completedTurn = client.turn;
  await client.close();
  await client.delivery;
  assert.ok(
    f.requests.some((r) =>
      r.payload.turn === completedTurn && r.payload.learn !== false
    ),
  );
});

test("context failure does not replay already acknowledged user evidence", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(f.config, f.descriptor);
  f.setContextAvailable(false);
  assert.match(await client.begin("Use fresh facts."), /unavailable/);
  await client.delivery;
  assert.equal(f.requests.filter((r) => r.path === "/v1/observe").length, 1);
  assert.equal(
    (await readdir(client.directory)).filter((n) => n.endsWith(".json")).length,
    0,
  );
  await client.close();
  await client.delivery;
  assert.equal(
    f.requests.filter((r) => r.path === "/v1/observe").length,
    2,
    "completed-turn learning is a different observation and must still be delivered",
  );
});

test("session extensions select the backend binding and switches", async (t) => {
  const f = await fixture(t);
  const home = join(f.root, "xdg");
  const previous = process.env.XDG_CONFIG_HOME;
  process.env.XDG_CONFIG_HOME = home;
  t.after(() => {
    if (previous === undefined) delete process.env.XDG_CONFIG_HOME;
    else process.env.XDG_CONFIG_HOME = previous;
  });
  const legacy = join(f.root, "legacy.json");
  await writeFile(legacy, JSON.stringify(f.config), { mode: 0o600 });
  const memory = {
    plugin: { id: "matrix", version: "1.0.0", digest: "sha256:x" },
    protocol: "matrix_http_v1",
    tools: ["memory_search", "memory_get"],
    recall: true,
    observe: true,
    enable_tools: true,
    enable_recall: false,
  };
  // Without a descriptor the legacy enrollment keeps every capability.
  const old = await memoryConfiguration("codex", {
    extensions: null,
    legacyPath: legacy,
  });
  assert.equal(old.enableRecall && old.enableTools && old.observe, true);
  assert.equal(memoryToolNames(old).length, 7);
  // The Matrix Plugin falls back to the legacy enrollment.
  const fallback = await memoryConfiguration("codex", {
    extensions: { schema: 1, memory },
    legacyPath: legacy,
  });
  assert.equal(fallback.enableRecall, false);
  assert.deepEqual(memoryToolNames(fallback), [
    "mcp__matrix__memory_search",
    "mcp__matrix__memory_get",
  ]);
  // The Cowboy binding path wins over the legacy file.
  const bound = join(home, "cowboy", "memory", "matrix");
  await mkdir(bound, { recursive: true, mode: 0o700 });
  await writeFile(
    join(bound, "codex.json"),
    JSON.stringify({ ...f.config, state_dir: join(f.root, "bound") }),
    { mode: 0o600 },
  );
  const preferred = await memoryConfiguration("codex", {
    extensions: { schema: 1, memory },
    legacyPath: legacy,
  });
  assert.equal(preferred.state_dir, join(f.root, "bound"));
  // Another backend never reads the Matrix enrollment.
  assert.equal(
    await memoryConfiguration("codex", {
      extensions: {
        schema: 1,
        memory: { ...memory, plugin: { ...memory.plugin, id: "other" } },
      },
      legacyPath: legacy,
    }),
    null,
  );
  // No backend, an unknown protocol, or everything off means no memory.
  for (
    const extensions of [
      { schema: 1, memory: null },
      { schema: 1, memory: { ...memory, protocol: "unknown" } },
      {
        schema: 1,
        memory: {
          ...memory,
          enable_tools: false,
          enable_recall: false,
          observe: false,
        },
      },
    ]
  ) {
    assert.equal(
      await memoryConfiguration("codex", { extensions, legacyPath: legacy }),
      null,
    );
  }
  // An invalid descriptor costs memory, never the launch.
  assert.equal(
    await memoryConfiguration("codex", {
      extensions: {
        schema: 1,
        memory: { ...memory, plugin: { id: "../escape" } },
      },
    }),
    null,
  );
});

test("a Machine without a local backend uses the session's relay", async (t) => {
  const f = await fixture(t);
  const memory = {
    plugin: { id: "matrix" },
    protocol: "matrix_http_v1",
    tools: ["memory_search"],
    enable_tools: true,
    enable_recall: true,
    observe: true,
    relay: {
      endpoint: f.config.endpoint,
      token: "cbm1.session-123." + "a".repeat(64),
      state_dir: join(f.root, "relay", "session-123"),
    },
  };
  const absent = join(f.root, "absent.json");
  const config = await memoryConfiguration("claude", {
    extensions: { schema: 1, memory },
    legacyPath: absent,
  });
  assert.equal(config.relay, true);
  assert.equal(config.server, "matrix");
  const client = await MatrixClient.open(config);
  assert.equal(client.mcp().headers.Authorization, "Bearer " + config.token);
  await client.close();
  await client.delivery;
  // A local enrollment stays direct: the relay adds a dependency.
  const local = join(f.root, "local.json");
  await writeFile(
    local,
    JSON.stringify({ ...f.config, provider: "claude" }),
    { mode: 0o600 },
  );
  const direct = await memoryConfiguration("claude", {
    extensions: { schema: 1, memory },
    legacyPath: local,
  });
  assert.equal(direct.relay, undefined);
  assert.equal(direct.token, f.config.token);
  // Only this Machine's loopback relay is ever accepted.
  assert.equal(
    await memoryConfiguration("claude", {
      extensions: {
        schema: 1,
        memory: {
          ...memory,
          relay: { ...memory.relay, endpoint: "http://example.com" },
        },
      },
      legacyPath: absent,
    }),
    null,
  );
});

test("an outage costs one wait, not one per turn", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(f.config, f.descriptor);
  f.setAvailable(false);
  assert.match(await client.begin("first"), /unavailable/);
  await client.delivery;
  const asked = f.requests.filter((r) => r.path === "/v1/context").length;
  const during = f.requests.length;
  assert.match(await client.begin("second"), /unavailable/);
  assert.equal(
    f.requests.filter((r) => r.path === "/v1/context").length,
    asked,
  );
  assert.ok(f.requests.length >= during);
  // A successful delivery closes the breaker and the queued turns arrive.
  f.setAvailable(true);
  await client.flush();
  assert.equal(client.unavailableUntil, 0);
  assert.match(await client.begin("third"), /Matrix memory\]/);
  await client.close();
  await client.delivery;
});

test("no memory failure reaches the agent", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(f.config, f.descriptor);
  // A full or unwritable queue: every enqueue fails.
  client.enqueue = () => Promise.reject(new Error("no space left on device"));
  assert.match(await client.begin("question"), /unavailable/);
  client.add("assistant", "answer");
  await client.finish();
  await client.close();
  await client.delivery;
});

test("a stalled service delays a turn by a bounded time", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(f.config, f.descriptor);
  client.request = () => new Promise(() => {});
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const reply = client.begin("question");
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  t.mock.timers.tick(5000);
  assert.match(await reply, /unavailable/);
  t.mock.timers.reset();
  delete client.request;
  client.inFlight.clear();
  await client.close();
  await client.delivery;
});

test("descriptor file must be private", async (t) => {
  const f = await fixture(t);
  const path = join(f.root, "extensions.json");
  await writeFile(path, JSON.stringify({ schema: 1, memory: null }), {
    mode: 0o600,
  });
  assert.deepEqual(await sessionExtensions({ path }), {
    schema: 1,
    memory: null,
  });
  const disabled = { schema: 1, memory: null, calls: null };
  await chmod(path, 0o644);
  assert.deepEqual(await sessionExtensions({ path }), disabled);
  await chmod(path, 0o600);
  await writeFile(path, "{not json");
  assert.deepEqual(await sessionExtensions({ path }), disabled);
  await writeFile(path, JSON.stringify({ schema: 2 }));
  assert.deepEqual(await sessionExtensions({ path }), disabled);
  assert.deepEqual(
    await sessionExtensions({ path: join(f.root, "absent.json") }),
    disabled,
  );
  assert.deepEqual(await sessionExtensions({ path: "relative" }), disabled);
  assert.equal(await sessionExtensions({ path: "" }), null);
});

test("recall off sends no context request; tools off omits the write hint", async (t) => {
  const f = await fixture(t);
  const quiet = await MatrixClient.open(
    { ...f.config, enableRecall: false, enableTools: true, observe: true },
    f.descriptor,
  );
  const text = await quiet.begin("Remember the release gate");
  assert.match(text, /Current user evidence for memory_put/);
  assert.equal(f.requests.some((r) => r.path === "/v1/context"), false);
  await quiet.close();
  const silent = await MatrixClient.open(
    { ...f.config, enableRecall: false, enableTools: false, observe: true },
    f.descriptor,
  );
  assert.equal(await silent.begin("Another turn"), "");
  silent.add("assistant", "Done");
  await silent.close();
  await silent.flush();
  const observed = f.requests.filter((r) => r.path === "/v1/observe");
  assert.ok(
    observed.some((r) =>
      r.payload.events?.some((e) => e.text === "Another turn")
    ),
  );
});

test("recall works for a backend that does not observe", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(
    { ...f.config, enableRecall: true, enableTools: false, observe: false },
    f.descriptor,
  );
  const text = await client.begin("What is the release gate?");
  assert.match(text, /Current revision only/);
  assert.equal(f.requests.some((r) => r.path === "/v1/observe"), false);
  await client.close();
  assert.deepEqual(await sessionExtensions({ path: "disabled" }), {
    schema: 1,
    memory: null,
    calls: null,
  });
});
