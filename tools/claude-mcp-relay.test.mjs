// Both relays native Claude may start for a target MCP server, the Node
// mcp-proxy.mjs and the packaged cowboy-mcp-relay, as real processes against
// a bridge that answers like mod-bridge.mjs. COWBOY_MCP_RELAY names the
// native binary; without it only the Node relay runs.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const proxy = fileURLToPath(
  new URL("../plugins/claude-code/runtime/mcp-proxy.mjs", import.meta.url),
);
const waiter = fileURLToPath(
  new URL("../plugins/claude-code/runtime/task-wait.mjs", import.meta.url),
);
const relays = [["node", [process.execPath, proxy]]];
const waiters = [["node", [process.execPath, waiter]]];
if (process.env.COWBOY_MCP_RELAY) {
  relays.push(["native", [process.env.COWBOY_MCP_RELAY]]);
  waiters.push(["native", [process.env.COWBOY_MCP_RELAY, "task-wait"]]);
}

async function bridge(t, route) {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-relay-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const socketPath = join(directory, "bridge.sock");
  const posts = [];
  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const answer = (status, value) => {
      // As mod-bridge.mjs: headers first, so the body is chunked.
      response.writeHead(status, { "content-type": "application/json" });
      response.end(JSON.stringify(value));
    };
    if (request.headers.authorization !== "Bearer secret") {
      answer(403, { deny: "Execution endpoint unavailable" });
      return;
    }
    const call = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    posts.push([request.url, call]);
    const [status, value] = await route(request.url, call, posts);
    answer(status, value);
  });
  await new Promise((resolve) => server.listen(socketPath, resolve));
  t.after(() => server.close());
  const context = join(directory, "context.json");
  await writeFile(
    context,
    JSON.stringify({ socketPath, bridgeToken: "secret" }),
  );
  return { context, posts };
}

function start(argv, context, server = "db") {
  const child = spawn(argv[0], [...argv.slice(1), context, server], {
    env: {
      PATH: process.env.PATH,
      CLAUDECODE: "1",
      CLAUDE_CODE_SESSION_ID: "s-1",
      HOME: "/nonexistent",
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = { stdout: "", stderr: "" };
  child.stdout.on("data", (data) => output.stdout += data);
  child.stderr.on("data", (data) => output.stderr += data);
  const exit = new Promise((resolve) => child.once("exit", resolve));
  return { child, output, exit };
}

const encode = (text) => Buffer.from(text).toString("base64");

for (const [name, argv] of relays) {
  test(`${name} relay carries stdio both ways and ends the target server`, async (t) => {
    let wrote;
    const written = new Promise((resolve) => wrote = resolve);
    const { context, posts } = await bridge(t, async (path) => {
      if (path === "/mcp-start") return [200, { id: "p1" }];
      if (path === "/mcp-write") {
        wrote();
        return [200, { status: "accepted" }];
      }
      if (path === "/mcp-read") {
        const reads = posts.filter(([url]) => url === "/mcp-read").length;
        if (reads === 1) {
          return [200, {
            chunks: [{ stream: "stdout", data: encode("out\n") }],
            afterSeq: 1,
            closed: false,
          }];
        }
        // The target answers only after native's message reached it.
        await written;
        return [200, {
          chunks: [{ stream: "stderr", data: encode("err\n") }],
          afterSeq: 2,
          closed: true,
          exitCode: 3,
        }];
      }
      return [200, { stopped: true }];
    });
    const relay = start(argv, context);
    relay.child.stdin.write('{"jsonrpc":"2.0"}\n');
    assert.equal(await relay.exit, 3);
    assert.deepEqual(relay.output, { stdout: "out\n", stderr: "err\n" });
    assert.deepEqual(posts[0], [
      "/mcp-start",
      { server: "db", env: { CLAUDECODE: "1", CLAUDE_CODE_SESSION_ID: "s-1" } },
    ]);
    assert.deepEqual(posts.find(([path]) => path === "/mcp-write")[1], {
      id: "p1",
      data: encode('{"jsonrpc":"2.0"}\n'),
    });
    assert.deepEqual(
      posts.filter(([path]) => path === "/mcp-read").map(([, value]) => value),
      [{ id: "p1", afterSeq: null }, { id: "p1", afterSeq: 1 }],
    );
    // The stop that follows the close may race the exit; it is never sent twice.
    assert.ok(posts.filter(([path]) => path === "/mcp-stop").length <= 1);
  });

  test(`${name} relay stops the target when native closes stdin`, async (t) => {
    let stopped;
    const stop = new Promise((resolve) => stopped = resolve);
    const { context, posts } = await bridge(t, async (path) => {
      if (path === "/mcp-start") return [200, { id: "p2" }];
      if (path === "/mcp-stop") {
        stopped();
        return [200, { stopped: true }];
      }
      if (path === "/mcp-read") {
        await stop;
        return [200, { chunks: [], afterSeq: 1, closed: true, exitCode: 0 }];
      }
      return [200, { status: "accepted" }];
    });
    const relay = start(argv, context);
    relay.child.stdin.end();
    assert.equal(await relay.exit, 0);
    // The Node relay stops again after the close; repeating it is harmless.
    const stops = posts.filter(([path]) => path === "/mcp-stop");
    assert.ok(stops.length >= 1);
    assert.ok(stops.every(([, value]) => value.id === "p2"));
  });

  test(`${name} relay ends the target on SIGTERM`, async (t) => {
    let reading;
    const read = new Promise((resolve) => reading = resolve);
    const { context, posts } = await bridge(t, async (path) => {
      if (path === "/mcp-start") return [200, { id: "p3" }];
      if (path === "/mcp-read") {
        reading();
        return await new Promise(() => {});
      }
      return [200, { stopped: true }];
    });
    const relay = start(argv, context);
    await read;
    relay.child.kill("SIGTERM");
    assert.equal(await relay.exit, 143);
    assert.deepEqual(posts.find(([path]) => path === "/mcp-stop"), [
      "/mcp-stop",
      { id: "p3" },
    ]);
  });

  test(`${name} relay reports a refused start`, async (t) => {
    const { context } = await bridge(
      t,
      async () => [400, { deny: "Invalid execution call" }],
    );
    const relay = start(argv, context);
    assert.equal(await relay.exit, 1);
    assert.match(
      relay.output.stderr,
      /^Target MCP server unavailable: .*Invalid execution call\n$/,
    );
  });
}

function wait(argv, context, job) {
  const child = spawn(argv[0], [...argv.slice(1), job], {
    env: { PATH: process.env.PATH, COWBOY_CLAUDE_CONTEXT: context },
    stdio: ["ignore", "ignore", "ignore"],
  });
  return { child, exit: new Promise((resolve) => child.once("exit", resolve)) };
}

for (const [name, argv] of waiters) {
  test(`${name} task waiter ends with the target command's status`, async (t) => {
    const { context, posts } = await bridge(t, async (_path, call) => {
      if (call.afterSeq === null) return [200, { afterSeq: 4 }];
      return [200, { closed: true, exitCode: 3 }];
    });
    const waiter = wait(argv, context, "job-1");
    assert.equal(await waiter.exit, 3);
    assert.deepEqual(posts.map(([, call]) => call), [
      { id: "job-1", afterSeq: null },
      { id: "job-1", afterSeq: 4 },
    ]);
  });

  test(`${name} task waiter outlives a stopped command`, async (t) => {
    const { context } = await bridge(t, async () => [200, { stopped: true }]);
    const waiter = wait(argv, context, "job-2");
    t.after(() => waiter.child.kill("SIGKILL"));
    const ended = await Promise.race([
      waiter.exit.then(() => true),
      new Promise((resolve) => setTimeout(() => resolve(false), 500)),
    ]);
    assert.equal(ended, false);
  });
}
