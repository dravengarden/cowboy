// Test-only fake execution endpoint: the subset of the Codex exec-server
// protocol the facade uses, served over WebSocket on a Unix socket against
// a local directory, with lost-reply injection. Not shipped.
import { spawn } from "node:child_process";
import {
  closeSync,
  lstatSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  readSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:http";
import { fileURLToPath, pathToFileURL } from "node:url";

// The execution helper the real executor exports to the processes it starts
// (built by `cargo build -p cowboy-machine --bin cowboy-execution-host`).
const FILE_HELPER = fileURLToPath(
  new URL("../../../../target/debug/cowboy-execution-host", import.meta.url),
);
import { WebSocketServer } from "ws";

// `virtual` presents `root` under another absolute path, so a test can give
// the proxy a target path that does not exist on the runtime side.
export async function fakeEndpoint(
  socketPath,
  root,
  {
    lose = () => false,
    fail = () => undefined,
    shell = "/bin/sh",
    virtual = root,
    fileHelper = FILE_HELPER,
    managed = undefined,
    // The executor's notifications naming a process (its output and its
    // end), as the worker forwards them; `false` drops every one.
    notifications = true,
    // Milliseconds each request waits before it is served.
    delay = () => 0,
  } = {},
) {
  const toReal = (path) => (path.startsWith(virtual) ? root + path.slice(virtual.length) : path);
  const toVirtual = (path) => (path.startsWith(root) ? virtual + path.slice(root.length) : path);
  const http = createServer();
  const wss = new WebSocketServer({ server: http, maxPayload: 14 * 1024 * 1024 });
  const calls = [];
  let inflight = 0;
  let maxInflight = 0;
  const notify = (method, processId) => {
    if (!notifications) return;
    for (const client of wss.clients) {
      client.send(JSON.stringify({ method, params: { processId } }));
    }
  };
  const processes = new Map();
  const handles = new Map();
  const missing = () => ({ code: -32004, message: "No such file or directory (os error 2)" });
  const path = (params) => toReal(fileURLToPath(params.path));
  const methods = {
    initialize: () => ({
      sessionId: "fake-session",
      environmentInfo: {
        executorVersion: "0.159.3",
        platformOs: "linux",
        cwd: pathToFileURL(virtual).href,
        shell: { path: shell },
      },
      // A managed keeper's announcement of the constraint it enforces.
      ...(managed ? { cowboyManaged: managed } : {}),
    }),
    "fs/canonicalize": (params) => ({
      path: pathToFileURL(toVirtual(realpathSync(path(params)))).href,
    }),
    "fs/getMetadata": (params) => {
      const link = lstatSync(path(params));
      const stat = statSync(path(params));
      return {
        isDirectory: stat.isDirectory(),
        isFile: stat.isFile(),
        isSymlink: link.isSymbolicLink(),
        size: stat.size,
      };
    },
    "fs/readFile": (params) => ({ dataBase64: readFileSync(path(params)).toString("base64") }),
    "fs/readDirectory": (params) => ({
      entries: readdirSync(path(params), { withFileTypes: true }).map((entry) => ({
        fileName: entry.name,
        isDirectory: entry.isDirectory(),
        isFile: entry.isFile(),
      })),
    }),
    "fs/open": (params) => {
      if (!statSync(path(params)).isFile()) throw { code: -32600, message: "not a file" };
      handles.set(params.handleId, openSync(path(params), "r"));
      return { handleId: params.handleId };
    },
    "fs/readBlock": (params) => {
      const buffer = Buffer.alloc(params.len);
      const read = readSync(handles.get(params.handleId), buffer, 0, params.len, params.offset);
      return { chunk: buffer.subarray(0, read).toString("base64"), eof: read === 0 };
    },
    "fs/close": (params) => {
      closeSync(handles.get(params.handleId));
      handles.delete(params.handleId);
      return {};
    },
    "fs/writeFile": (params) => {
      writeFileSync(path(params), Buffer.from(params.dataBase64, "base64"));
      return {};
    },
    "fs/createDirectory": (params) => {
      mkdirSync(path(params), { recursive: params.recursive });
      return {};
    },
    "fs/remove": (params) => {
      rmSync(path(params), { recursive: params.recursive, force: params.force });
      return {};
    },
    "process/start": (params) => {
      const child = spawn(params.argv[0], params.argv.slice(1).map(toReal), {
        cwd: toReal(fileURLToPath(params.cwd)),
        stdio: ["ignore", "pipe", "pipe"],
        env: { ...process.env, COWBOY_EXECUTION_FILE_HELPER: fileHelper ?? "" },
      });
      const entry = {
        chunks: [],
        seq: 0,
        exited: false,
        exitCode: null,
        closed: false,
        waiters: [],
      };
      const wake = (method = "process/output") => {
        for (const waiter of entry.waiters.splice(0)) waiter();
        notify(method, params.processId);
      };
      for (const stream of ["stdout", "stderr"]) {
        child[stream].on("data", (bytes) => {
          entry.chunks.push({ seq: ++entry.seq, stream, chunk: bytes.toString("base64") });
          wake();
        });
      }
      child.on("error", () => {
        entry.exited = entry.closed = true;
        entry.exitCode = 127;
        wake("process/exited");
      });
      child.on("close", (code, signal) => {
        entry.exited = entry.closed = true;
        entry.exitCode = code ?? (signal ? 128 + 15 : null);
        wake("process/exited");
      });
      entry.child = child;
      processes.set(params.processId, entry);
      return { processId: params.processId };
    },
    "process/read": async (params) => {
      const entry = processes.get(params.processId);
      if (!entry) throw { code: -32600, message: `unknown process id ${params.processId}` };
      const after = params.afterSeq ?? 0;
      const pending = () => entry.chunks.filter((chunk) => chunk.seq > after);
      if (!pending().length && !entry.closed) {
        await new Promise((done) => {
          entry.waiters.push(done);
          setTimeout(done, params.waitMs);
        });
      }
      return {
        chunks: pending(),
        exited: entry.exited,
        exitCode: entry.exitCode,
        closed: entry.closed,
      };
    },
    // The worker's own requests: a session without extensions.
    "cowboy/extensions": () => ({ tools: [], instructions: "" }),
    "process/terminate": (params) => {
      processes.get(params.processId)?.child.kill("SIGTERM");
      return {};
    },
    // Answered by the worker endpoint itself (its ledger), never the target.
    "cowboy/uncertainEffect": () => ({}),
  };
  wss.on("connection", (socket) => {
    socket.on("message", async (bytes) => {
      const frame = JSON.parse(bytes.toString());
      if (frame.id === undefined) return;
      calls.push({ method: frame.method, params: frame.params, at: Date.now() });
      let reply;
      inflight += 1;
      maxInflight = Math.max(maxInflight, inflight);
      try {
        const wait = delay(frame.method, frame.params);
        if (wait) await new Promise((done) => setTimeout(done, wait));
        const method = methods[frame.method];
        if (!method) throw { code: -32601, message: "unknown method" };
        const failure = fail(frame.method, frame.params);
        if (failure) throw failure;
        reply = { id: frame.id, result: await method(frame.params) };
      } catch (error) {
        reply = {
          id: frame.id,
          error:
            error?.code === "ENOENT"
              ? missing()
              : {
                  code: Number.isInteger(error?.code) ? error.code : -32600,
                  message: String(error?.message),
                  ...(error?.data ? { data: error.data } : {}),
                },
        };
      }
      inflight -= 1;
      if (lose(frame.method, frame.params)) return socket.terminate();
      socket.send(JSON.stringify(reply));
    });
  });
  await new Promise((done) => http.listen(socketPath, done));
  return {
    calls,
    /** The most requests this endpoint was serving at once. */
    maxInflight: () => maxInflight,
    close: () => {
      for (const entry of processes.values()) entry.child.kill("SIGKILL");
      for (const client of wss.clients) client.terminate();
      http.close();
    },
  };
}
