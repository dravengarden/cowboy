// Cowboy Agent API program runner. It runs inside the Controller's contained
// runtime with no network, files or environment: one NDJSON request per API
// call on stdout, one reply per line on stdin, then exactly one final frame.
const MAX_LOG = 4096;
const out = (frame) => process.stdout.write(JSON.stringify(frame) + "\n");
// The final frame must be flushed before exit, or a pipe truncates it.
const finish = (frame) =>
  new Promise((resolve) =>
    process.stdout.write(JSON.stringify(frame) + "\n", resolve)
  )
    .then(() => process.exit(0));
const lines = console[Symbol.asyncIterator]();
const pending = new Map();
let next = 1;
let logs = "";
let logBytes = 0;

function log(...values) {
  if (logBytes >= MAX_LOG) return;
  const line = values.map((value) =>
    typeof value === "string" ? value : (() => {
      try {
        return JSON.stringify(value);
      } catch {
        return String(value);
      }
    })()
  ).join(" ");
  const bytes = Buffer.from(line + "\n");
  let count = Math.min(bytes.length, MAX_LOG - logBytes);
  // Keep a whole UTF-8 prefix. Slicing UTF-16 can leave a lone surrogate,
  // producing a final JSON frame the broker cannot decode.
  while (count < bytes.length && count > 0 && (bytes[count] & 0xc0) === 0x80) {
    count--;
  }
  logs += bytes.toString("utf8", 0, count);
  logBytes += count;
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
  control: namespace("control", {
    list: (domain) => domain === undefined ? {} : ({ domain }),
    describe: (name) => ({ name }),
    read: (name, input, selection) => ({
      name,
      input: input ?? {},
      ...(selection === undefined ? {} : { selection }),
    }),
  }),
  api: namespace("api", { help: (topic) => ({ topic }) }),
  preferences: namespace("preferences", {
    schema: none,
    get: (scope) => ({ scope }),
  }),
  accounts: namespace("accounts", {
    transition_status: (operation) => ({ operation }),
    list: none,
    login_options: (account_id, machine_id) => ({ account_id, machine_id }),
  }),
  session: namespace("session", {
    info: none,
    usage: none,
    queue: query,
    drafts: query,
    message: (id, q) => ({ ...q, id }),
    events: query,
    history: query,
    event: (seq, q) => ({ ...q, seq }),
    search: query,
  }),
  calls: namespace("calls", {
    list: query,
    capabilities: query,
    inspect: callId,
    wait: (call_id, timeout_ms) => ({ call_id, timeout_ms }),
    result: (call_id, q) => ({ ...q, call_id }),
    observe: (request_id) => ({ request_id }),
  }),
  sessions: namespace("sessions", {
    list: query,
    info: id,
    usage: id,
    queue: (session_id, q) => ({ ...q, session_id }),
    drafts: (session_id, q) => ({ ...q, session_id }),
    message: (session_id, id, q) => ({ ...q, session_id, id }),
    events: (session_id, q) => ({ ...q, session_id }),
    history: (session_id, q) => ({ ...q, session_id }),
    event: (session_id, seq, q) => ({ ...q, session_id, seq }),
    search: (session_id, q) => ({ ...q, session_id }),
  }),
  usage: namespace("usage", {
    list: query,
    read: (account, q) => ({ ...q, account }),
    diagnostics: query,
  }),
  machines: namespace("machines", {
    list: query,
    read: (id, q) => ({ ...q, id }),
  }),
  configuration: namespace("configuration", {
    schema: (scope, q) => ({ ...q, scope }),
    read: (scope, q) => ({ ...q, scope }),
  }),
  documents: namespace("documents", {
    list: query,
    read: (id, q) => ({ ...q, id }),
  }),
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
  await finish(failure(error, "compile_error"));
}
let frame;
try {
  const value = await program({ cowboy });
  try {
    JSON.stringify(value);
    frame = { done: true, value: value === undefined ? null : value, logs };
  } catch (error) {
    frame = failure(error, "result_not_json");
  }
} catch (error) {
  frame = failure(error, "program_error");
}
await finish(frame);
