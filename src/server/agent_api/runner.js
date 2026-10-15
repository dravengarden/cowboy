// Cowboy Agent API program runner. It runs inside the Controller's contained
// runtime with no network, files or environment: one NDJSON request per API
// call on stdout, one reply per line on stdin, then exactly one final frame.
const MAX_LOG = 4096;
const out = (frame) => process.stdout.write(JSON.stringify(frame) + "\n");
const lines = console[Symbol.asyncIterator]();
const pending = new Map();
let next = 1;
let logs = "";

function log(...values) {
  if (logs.length >= MAX_LOG) return;
  const line = values.map((value) =>
    typeof value === "string" ? value : (() => {
      try {
        return JSON.stringify(value);
      } catch {
        return String(value);
      }
    })()
  ).join(" ");
  logs = (logs + line + "\n").slice(0, MAX_LOG);
}
for (const name of ["log", "info", "warn", "error", "debug"]) {
  console[name] = log;
}

(async () => {
  for await (const line of lines) {
    let reply;
    try {
      reply = JSON.parse(line);
    } catch {
      continue;
    }
    const waiter = pending.get(reply.id);
    if (!waiter) continue;
    pending.delete(reply.id);
    if (reply.error) {
      const error = new Error(reply.error.message ?? reply.error.code);
      error.code = reply.error.code;
      waiter.reject(error);
    } else {
      waiter.resolve(reply.ok);
    }
  }
})();

function call(op, args) {
  const id = next++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    out({ id, op, args });
  });
}

const namespace = (name, methods) =>
  Object.freeze(Object.fromEntries(
    Object.entries(methods).map(([method, shape]) => [
      method,
      (...values) => call(`${name}.${method}`, shape(...values)),
    ]),
  ));
const none = () => ({});
const id = (id) => ({ id });
const callId = (call_id) => ({ call_id });
const query = (q) => q ?? {};
const cowboy = Object.freeze({
  session: namespace("session", {
    info: none,
    queue: none,
    drafts: none,
    events: query,
  }),
  calls: namespace("calls", {
    capabilities: none,
    inspect: callId,
    wait: (call_id, timeout_ms) => ({ call_id, timeout_ms }),
    result: callId,
    observe: (request_id) => ({ request_id }),
  }),
  sessions: namespace("sessions", { list: query, info: id }),
  documents: namespace("documents", { list: none, read: id }),
});

function failure(error, code) {
  return {
    done: false,
    error: {
      code: typeof error?.code === "string" ? error.code : code,
      message: String(error?.message ?? error).slice(0, 2000),
    },
    logs,
  };
}

let program;
try {
  program = (await import(process.argv[2])).default;
  if (typeof program !== "function") throw new Error("no default function");
} catch (error) {
  out(failure(error, "compile_error"));
  process.exit(0);
}
try {
  const value = await program({ cowboy });
  try {
    out({ done: true, value: value === undefined ? null : value, logs });
  } catch (error) {
    out(failure(error, "result_not_json"));
  }
} catch (error) {
  out(failure(error, "program_error"));
}
process.exit(0);
