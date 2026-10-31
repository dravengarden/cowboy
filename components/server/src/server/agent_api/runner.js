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

// Paged lists return {items-key, cursor}; agents habitually treat them as
// arrays. Non-enumerable delegates keep the JSON shape unchanged while
// `.map`, `.filter`, `for...of` and `.length` reach the primary array.
const LIST_KEYS = {
  "calls.list": ["calls"],
  "machines.list": ["machines"],
  "sessions.list": ["sessions"],
  "usage.list": ["accounts"],
  "control.list": ["operations", "domains"],
};
const ARRAY_METHODS = [
  "map",
  "filter",
  "find",
  "findIndex",
  "some",
  "every",
  "forEach",
  "reduce",
  "flatMap",
  "slice",
  "includes",
  "at",
];
function listLike(op, value) {
  const key = LIST_KEYS[op]?.find((key) => Array.isArray(value?.[key]));
  if (!key) return value;
  const items = value[key];
  const hidden = (name, descriptor) =>
    Object.defineProperty(value, name, { enumerable: false, ...descriptor });
  for (const name of ARRAY_METHODS) {
    if (!(name in value)) {
      hidden(name, { value: (...args) => items[name](...args) });
    }
  }
  hidden("length", { get: () => items.length });
  hidden(Symbol.iterator, { value: () => items[Symbol.iterator]() });
  return value;
}

function call(op, args) {
  const id = next++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve: (value) => resolve(listLike(op, value)), reject });
    out({ id, op, args });
  });
}

// A misspelled operation otherwise surfaces as an engine TypeError
// ("x is not a function") that names no valid alternative.
const PASSTHROUGH = new Set(["then", "toJSON", "constructor"]);
const strict = (object, path, hint) =>
  new Proxy(object, {
    get(target, key, receiver) {
      if (typeof key !== "string" || key in target || PASSTHROUGH.has(key)) {
        return Reflect.get(target, key, receiver);
      }
      const error = new Error(
        `${path}.${key} does not exist; ${path} has: ${
          Object.keys(target).join(", ")
        }${hint}`,
      );
      error.code = "unknown_operation";
      throw error;
    },
  });
const namespace = (name, methods) =>
  strict(
    Object.freeze(Object.fromEntries(
      Object.entries(methods).map(([method, shape]) => [
        method,
        (...values) => call(`${name}.${method}`, shape(...values)),
      ]),
    )),
    `cowboy.${name}`,
    "",
  );
const none = () => ({});
const query = (q) => q ?? {};
// Agents often write `info({id})` or `history({session_id, limit})` for a
// positional id. That form was always a type error, so accepting it (and
// the obvious alias key) cannot change what a valid call means.
const plain = (value) =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const keyed = (key, aliases, shape = (value, q) => ({ ...q, [key]: value })) =>
(first, ...rest) => {
  if (!plain(first)) return shape(first, ...rest);
  const args = { ...(plain(rest[0]) ? rest[0] : {}), ...first };
  for (const alias of aliases) {
    if (!(key in args) && alias in args) {
      args[key] = args[alias];
      delete args[alias];
    }
  }
  return args;
};
const id = keyed("id", ["session_id"]);
const callId = keyed("call_id", ["id"]);
const sessionId = keyed("session_id", ["id"]);
const cowboy = strict(Object.freeze({
  control: namespace("control", {
    list: keyed("domain", [], (domain) =>
      domain === undefined ? {} : ({ domain })),
    describe: keyed("name", []),
    read: (name, input, selection) => ({
      name,
      input: input ?? {},
      ...(selection === undefined ? {} : { selection }),
    }),
  }),
  api: namespace("api", { help: keyed("topic", []) }),
  preferences: namespace("preferences", {
    schema: none,
    get: (scope) => ({ scope }),
  }),
  accounts: namespace("accounts", {
    usage: (account_id) => ({ account_id }),
    revocation_status: (account_id) => ({ account_id }),
    transition_status: (operation) => ({ operation }),
    list: none,
    login_options: (account_id, machine_id) => ({ account_id, machine_id }),
    logins: (account_id, q) => ({ ...q, account_id }),
    login_status: (account_id, request_id) => ({ account_id, request_id }),
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
    wait: keyed("call_id", ["id"], (call_id, timeout_ms) =>
      plain(timeout_ms)
        ? ({ ...timeout_ms, call_id })
        : ({ call_id, timeout_ms })),
    result: callId,
    observe: (request_id) => ({ request_id }),
  }),
  sessions: namespace("sessions", {
    list: query,
    info: id,
    usage: id,
    queue: sessionId,
    drafts: sessionId,
    message: (session_id, id, q) => ({ ...q, session_id, id }),
    events: sessionId,
    history: sessionId,
    event: (session_id, seq, q) => ({ ...q, session_id, seq }),
    search: sessionId,
  }),
  usage: namespace("usage", {
    list: query,
    read: keyed("account", ["account_id", "id"]),
    diagnostics: query,
  }),
  machines: namespace("machines", {
    list: query,
    read: keyed("id", ["machine", "machine_id"]),
  }),
  configuration: namespace("configuration", {
    schema: (scope, q) => ({ ...q, scope }),
    read: (scope, q) => ({ ...q, scope }),
  }),
  documents: namespace("documents", {
    list: query,
    read: keyed("id", ["document_id"]),
  }),
}), "cowboy", "; help is cowboy.api.help(topic)");

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
