import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import test from "node:test";
import {
  expandVariables,
  loopbackUrl,
  mcpToolPrefix,
  nativeMcpServers,
  projectConfigDirectories,
  targetMcpServers,
} from "../plugins/claude-code/runtime/mcp.mjs";
import { relay } from "../plugins/claude-code/runtime/mcp-proxy.mjs";

const stdio = (name) => ({
  command: "python3",
  args: ["server.py", name, "${FOO:-dflt}", "${MISSING}"],
  env: { FOO: "${FOO}-x" },
});

test("servers load in native's scopes and precedence", () => {
  const userConfig = JSON.stringify({
    mcpServers: { dup: stdio("user"), usersrv: stdio("user"), off: stdio("u") },
    projects: {
      "/w/repo": {
        mcpServers: { dup: stdio("local"), localsrv: stdio("local") },
        disabledMcpjsonServers: ["hidden"],
        disabledMcpServers: ["off"],
      },
      "/w/repo/sub": { mcpServers: { wrongkey: stdio("x") } },
    },
  });
  const projectConfigs = [
    JSON.stringify({ mcpServers: { dup: stdio("outer"), outer: stdio("o") } }),
    JSON.stringify({
      mcpServers: { dup: stdio("project"), hidden: stdio("h") },
    }),
    "not json",
  ];
  const { entries, omitted } = targetMcpServers({
    userConfig,
    projectConfigs,
    cwd: "/w/repo/sub",
    repositoryRoot: "/w/repo",
    environment: { FOO: "foo" },
  });
  assert.deepEqual(
    entries.map((entry) => `${entry.scope}:${entry.name}`),
    ["local:dup", "user:usersrv", "project:outer", "local:localsrv"],
  );
  assert.deepEqual(entries[0].argv, [
    "python3",
    "server.py",
    "local",
    "foo",
    "${MISSING}",
  ]);
  assert.deepEqual(entries[0].env, { FOO: "foo-x" });
  assert.equal(entries[0].placement, "target");
  assert.deepEqual(omitted, []);
  // Outside a repository the working directory is the key.
  assert.deepEqual(
    targetMcpServers({
      userConfig,
      projectConfigs: [],
      cwd: "/w/repo/sub",
      environment: {},
    }).entries.map((entry) => entry.name),
    ["dup", "usersrv", "off", "wrongkey"],
  );
});

test("remote servers are reached from here unless they name the target", () => {
  const { entries, omitted } = targetMcpServers({
    userConfig: JSON.stringify({
      mcpServers: {
        api: {
          type: "http",
          url: "https://${HOST}/mcp",
          headers: { Authorization: "Bearer ${TOKEN}" },
        },
        events: { type: "sse", url: "https://example.com/sse" },
        local: { type: "http", url: "http://127.0.0.1:3000/mcp" },
        helper: {
          type: "http",
          url: "https://example.com/h",
          headersHelper: "get-token",
        },
        socket: { type: "ws", url: "wss://example.com" },
        unset: {
          type: "http",
          url: "https://example.com/x",
          headers: { Authorization: "Bearer ${RUNTIME_ONLY}" },
        },
        broken: { type: "stdio", args: ["x"] },
        "bad name": stdio("x"),
      },
    }),
    projectConfigs: [],
    cwd: "/p",
    environment: { HOST: "mcp.example.com", TOKEN: "t" },
  });
  assert.deepEqual(entries.map((entry) => entry.config), [
    {
      type: "http",
      url: "https://mcp.example.com/mcp",
      headers: { Authorization: "Bearer t" },
    },
    { type: "sse", url: "https://example.com/sse" },
  ]);
  assert.deepEqual(
    omitted.map((entry) => entry.name),
    ["local", "helper", "socket", "unset", "broken", "bad name"],
  );
  assert.deepEqual(
    nativeMcpServers(
      [...entries, {
        name: "db",
        placement: "target",
        argv: ["db"],
        env: {},
      }],
      { command: "/node", args: ["/stage/mcp-proxy.mjs", "/stage/ctx.json"] },
    ).db,
    {
      type: "stdio",
      command: "/node",
      args: ["/stage/mcp-proxy.mjs", "/stage/ctx.json", "db"],
    },
  );
});

test("expansion, loopback names and tool prefixes follow native", () => {
  assert.equal(expandVariables("${A}${B:-b}${C}", { A: "a" }), "ab${C}");
  assert.equal(expandVariables("${A:-}", {}), "");
  assert.ok(loopbackUrl("http://localhost:1/") && loopbackUrl("http://[::1]/"));
  assert.ok(!loopbackUrl("https://example.com/"));
  for (
    const url of [
      "http://localhost.:3000/mcp",
      "http://[::ffff:127.0.0.1]:3000/mcp",
      "http://[::]:1/",
      "http://app.localhost./",
    ]
  ) assert.ok(loopbackUrl(url), url);
  assert.ok(!loopbackUrl("http://[::ffff:10.0.0.1]/"));
  assert.equal(mcpToolPrefix("my-db_1"), "mcp__my-db_1__");
  assert.deepEqual(projectConfigDirectories("/a/b"), ["/a", "/a/b"]);
});

test("the relay carries stdio both ways and ends the target server", async () => {
  const posts = [];
  const reads = [
    {
      chunks: [{
        stream: "stdout",
        data: Buffer.from("out\n").toString("base64"),
      }],
      afterSeq: 1,
      closed: false,
    },
    {
      chunks: [{
        stream: "stderr",
        data: Buffer.from("err\n").toString("base64"),
      }],
      afterSeq: 2,
      closed: true,
      exitCode: 3,
    },
  ];
  const input = new PassThrough();
  const written = { stdout: [], stderr: [] };
  const send = async (path, value) => {
    posts.push([path, value]);
    if (path === "/mcp-start") return { id: "p1" };
    if (path === "/mcp-write") return { status: "accepted" };
    if (path === "/mcp-read") {
      await new Promise((resolve) => setTimeout(resolve, 10));
      return reads.shift();
    }
    return { stopped: true };
  };
  input.write('{"jsonrpc":"2.0"}\n');
  const code = await relay("db", {
    send,
    input,
    stdout: { write: (data) => written.stdout.push(String(data)) },
    stderr: { write: (data) => written.stderr.push(String(data)) },
    environment: {
      CLAUDECODE: "1",
      CLAUDE_CODE_SESSION_ID: "s-1",
      HOME: "/x",
    },
  });
  assert.equal(code, 3);
  assert.deepEqual(written, { stdout: ["out\n"], stderr: ["err\n"] });
  assert.deepEqual(posts[0], [
    "/mcp-start",
    { server: "db", env: { CLAUDECODE: "1", CLAUDE_CODE_SESSION_ID: "s-1" } },
  ]);
  assert.deepEqual(posts.find(([path]) => path === "/mcp-write")[1], {
    id: "p1",
    data: Buffer.from('{"jsonrpc":"2.0"}\n').toString("base64"),
  });
  assert.deepEqual(
    posts.filter(([path]) => path === "/mcp-read").map(([, value]) => value),
    [{ id: "p1", afterSeq: null }, { id: "p1", afterSeq: 1 }],
  );
  assert.deepEqual(posts.at(-1), ["/mcp-stop", { id: "p1" }]);
  input.end();
});

test("MCP traffic queues for its own few connection slots", async () => {
  const { WorkspaceTools } = await import(
    "../plugins/claude-code/runtime/tools.mjs"
  );
  let active = 0;
  let peak = 0;
  const tools = new WorkspaceTools(
    {
      call: async () => {
        peak = Math.max(peak, ++active);
        await new Promise((resolve) => setTimeout(resolve, 5));
        active--;
        return { ok: true };
      },
    },
    { workspace: { cwd: "/t" }, environment: { id: "e" } },
    "/nonexistent",
  );
  const results = await Promise.all(
    Array.from({ length: 12 }, () => tools.mcpCall("process/read", {})),
  );
  assert.equal(results.length, 12);
  assert.equal(peak, 4);
});

test("a gap in retained output ends the server instead of cutting its stream", async () => {
  const { WorkspaceTools } = await import(
    "../plugins/claude-code/runtime/tools.mjs"
  );
  const calls = [];
  const tools = new WorkspaceTools(
    {
      call: async (method, params) => {
        calls.push(method);
        if (method === "process/terminate") {
          const error = new Error("Target operation failed");
          error.remote = { message: "unknown process id p" };
          throw error;
        }
        return {
          chunks: [{ seq: 5, stream: "stdout", chunk: "eA==" }],
          closed: false,
        };
      },
    },
    { workspace: { cwd: "/t" }, environment: { id: "e" } },
    "/nonexistent",
  );
  tools.state.mcp = { p: "db" };
  tools.save = async (update) => update?.();
  assert.deepEqual(await tools.mcpRead("p", 2, 10), {
    chunks: [],
    afterSeq: 2,
    closed: true,
    exitCode: 1,
    lost: true,
  });
  assert.ok(calls.includes("process/terminate"));
  assert.deepEqual(tools.state.mcp, {});
  // A first read must start at the first number too.
  tools.state.mcp = { p: "db" };
  assert.equal((await tools.mcpRead("p", null, 10)).lost, true);
});

test("after its exit a server's output may skip only the exit's number", async () => {
  const { WorkspaceTools } = await import(
    "../plugins/claude-code/runtime/tools.mjs"
  );
  let reply;
  const tools = new WorkspaceTools(
    {
      call: async (method) => {
        if (method === "process/terminate") return {};
        return reply;
      },
    },
    { workspace: { cwd: "/t" }, environment: { id: "e" } },
    "/nonexistent",
  );
  tools.save = async (update) => update?.();
  tools.state.mcp = { p: "db" };
  const chunk = (seq) => ({ seq, stream: "stdout", chunk: "eA==" });
  // Without its exit notification, a gap after exit is lost output.
  reply = { chunks: [chunk(3), chunk(5)], exited: true, closed: false };
  assert.equal((await tools.mcpRead("p", 2, 10)).lost, true);
  tools.state.mcp = { p: "db" };
  tools.mcpExits = new Map([["p", 4]]);
  assert.equal((await tools.mcpRead("p", 2, 10)).afterSeq, 5);
  // An exit notification arriving after the read's reply still counts.
  tools.mcpExits = new Map();
  setTimeout(() => tools.mcpExits.set("p", 4), 100);
  assert.equal((await tools.mcpRead("p", 2, 10)).afterSeq, 5);
  reply = { chunks: [chunk(7)], exited: true, closed: false };
  assert.equal((await tools.mcpRead("p", 5, 10)).lost, true);
  tools.state.mcp = { q: "db" };
  reply = { chunks: [chunk(1), chunk(9)], exited: true, closed: false };
  assert.equal((await tools.mcpRead("q", null, 10)).lost, true);
});
