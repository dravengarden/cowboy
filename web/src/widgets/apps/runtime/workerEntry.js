// The trusted runtime Worker (docs/widget-apps.md §3, §5). It hosts one
// QuickJS VM that runs the app, and is the only code that talks to the host.
// Everything crossing from the VM is validated and bounded here before the
// host ever sees it; the VM is metered so it cannot monopolize this thread.
import { newQuickJSWASMModuleFromVariant, newVariant } from "quickjs-emscripten-core";
import RELEASE_SYNC from "@jitl/quickjs-wasmfile-release-sync";
import { ELEMENT_NAMES, LIMITS } from "./limits.js";

for (const name of [
  "fetch",
  "XMLHttpRequest",
  "WebSocket",
  "EventSource",
  "importScripts",
  "indexedDB",
  "caches",
  "BroadcastChannel",
  "Worker",
  "SharedWorker",
]) {
  try {
    Object.defineProperty(self, name, { value: undefined, configurable: false });
  } catch {}
}

const elements = new Set(ELEMENT_NAMES);
let port = null;
let vm = null;
let runtime = null;
let stopped = false;
const window = { start: 0, used: 0, throttled: 0, minuteStart: 0, minuteUsed: 0 };
const tree = { parent: new Map([[0, -1]]), depth: new Map([[0, 0]]) };
let credits = LIMITS.creditWindow;
const pending = [];
let pendingBytes = 0;
const logs = { windowStart: 0, count: 0 };

const post = (message) => port && port.postMessage(message);
function stop(reason) {
  if (stopped) return;
  stopped = true;
  post({ t: "stopped", reason });
  try {
    vm?.dispose();
    runtime?.dispose();
  } catch {}
  self.close();
}

/** Ends the app with a person-readable reason for a VM error message. */
function fail(message) {
  stop(
    message === "interrupted"
      ? "it ran too long"
      : message.includes("out of memory")
        ? "it used too much memory"
        : message.slice(0, 500),
  );
}

function within(limit, value, message) {
  if (value > limit) throw new Error(message);
}

/** Validates one batch from the VM and updates the shadow tree. */
function admit(json) {
  within(LIMITS.batchBytes, json.length, "a render update was too large");
  const ops = JSON.parse(json);
  if (!Array.isArray(ops)) throw new Error("malformed render update");
  for (const op of ops) {
    const [kind, id] = op;
    if (kind === "create") {
      if (!elements.has(op[2])) throw new Error(`unknown component ${String(op[2])}`);
      tree.parent.set(id, -1);
    } else if (kind === "text") {
      if (typeof op[2] !== "string") throw new Error("malformed text");
      within(LIMITS.textChars, op[2].length, "a text node was too long");
      if (!tree.parent.has(id)) tree.parent.set(id, -1);
    } else if (kind === "append" || kind === "before") {
      const child = op[2];
      if (!tree.parent.has(child) || !tree.parent.has(id)) throw new Error("unknown node");
      tree.parent.set(child, id);
      const depth = (tree.depth.get(id) ?? 0) + 1;
      within(LIMITS.depth, depth, "the app nested too deeply");
      tree.depth.set(child, depth);
    } else if (kind === "remove") {
      tree.parent.delete(id);
      tree.depth.delete(id);
    } else if (kind !== "props" && kind !== "clear") {
      throw new Error("malformed render update");
    }
  }
  within(LIMITS.nodes, tree.parent.size, "the app rendered too many components");
  pending.push(json);
  pendingBytes += json.length;
  if (pending.length > LIMITS.pendingBatches || pendingBytes > LIMITS.pendingBytes) {
    throw new Error("the app updated faster than it could be drawn");
  }
  pump();
}

function pump() {
  while (credits > 0 && pending.length > 0) {
    const json = pending.shift();
    pendingBytes -= json.length;
    credits -= 1;
    post({ t: "ops", json });
  }
}

/** Runs VM code under the per-call deadline and the rolling CPU windows.
 *  Loading the runtime and the first render (`startup`) have their own
 *  deadlines and count toward the minute, not the per-second window.
 *  Returns `{ ok, value }`; `ok` is false when throttled or stopped. */
function run(code, deadlineMs, startup = false) {
  const now = Date.now();
  if (now - window.start >= 1000) {
    window.throttled = window.used >= LIMITS.cpuPerSecondMs ? window.throttled + 1 : 0;
    window.start = now;
    window.used = 0;
  }
  if (now - window.minuteStart >= 60_000) {
    window.minuteStart = now;
    window.minuteUsed = 0;
  }
  if (window.throttled >= 3) {
    stop("it kept the processor busy");
    return { ok: false };
  }
  const allowance = startup
    ? deadlineMs
    : Math.min(deadlineMs, LIMITS.cpuPerSecondMs - window.used);
  if (allowance <= 0) return { ok: false };
  const until = now + allowance;
  runtime.setInterruptHandler(() => Date.now() > until);
  const result = vm.evalCode(code);
  let jobs = 0;
  while (!result.error && jobs < LIMITS.jobsPerTurn && runtime.hasPendingJob()) {
    const job = runtime.executePendingJobs(1);
    job.dispose?.();
    jobs += 1;
  }
  const spent = Date.now() - now;
  if (!startup) window.used += spent;
  window.minuteUsed += spent;
  if (result.error) {
    const error = vm.dump(result.error);
    result.error.dispose();
    fail(String(error?.message ?? error));
    return { ok: false };
  }
  const value = vm.dump(result.value);
  result.value.dispose();
  if (window.minuteUsed > LIMITS.cpuPerMinuteMs) {
    stop("it kept the processor busy");
    return { ok: false };
  }
  return { ok: true, value };
}

function hostFunctions() {
  const emit = vm.newFunction("__emit", (handle) => {
    try {
      admit(vm.getString(handle));
    } catch (error) {
      stop(String(error.message ?? error));
    }
  });
  const send = vm.newFunction("__send", (handle) => {
    const text = vm.getString(handle);
    if (text.length <= LIMITS.messageChars) post({ t: "send", text });
    else post({ t: "log", text: "send(): message too long" });
  });
  const log = vm.newFunction("__log", (handle) => {
    const now = Date.now();
    if (now - logs.windowStart >= 1000) {
      logs.windowStart = now;
      logs.count = 0;
    }
    logs.count += 1;
    if (logs.count <= LIMITS.logsPerSecond)
      post({ t: "log", text: vm.getString(handle).slice(0, 1024) });
  });
  const failed = vm.newFunction("__fail", (handle) => fail(vm.getString(handle)));
  for (const [name, fn] of [
    ["__emit", emit],
    ["__send", send],
    ["__log", log],
    ["__fail", failed],
  ]) {
    vm.setProp(vm.global, name, fn);
    fn.dispose();
  }
}

const literal = (value) => JSON.stringify(value);
let timersLeft = 0;
function drainTimers() {
  if (stopped || pending.length >= LIMITS.pendingBatches || timersLeft === 0) return;
  const result = run(`__cowboyTimersRun(${LIMITS.timersPerTurn})`, LIMITS.eventMs);
  if (result.ok) timersLeft = Number(result.value) || 0;
  if (timersLeft > 0) setTimeout(drainTimers, 16);
}

async function boot(data) {
  const module = await newQuickJSWASMModuleFromVariant(
    newVariant(RELEASE_SYNC, { wasmBinary: data.wasm }),
  );
  runtime = module.newRuntime();
  runtime.setMemoryLimit(LIMITS.memoryBytes);
  runtime.setMaxStackSize(LIMITS.stackBytes);
  vm = runtime.newContext();
  hostFunctions();
  if (!run(data.vm, LIMITS.loadMs, true).ok) return;
  post({ t: "nonce", nonce: data.nonce });
  run(
    `__cowboyStart(${literal(data.code)}, ${literal(JSON.stringify(data.props))}, ${literal(JSON.stringify(data.context))})`,
    LIMITS.firstRenderMs,
    true,
  );
  timersLeft = 1;
  drainTimers();
}

function onHost(event) {
  const message = event.data;
  if (stopped || !message || typeof message !== "object") return;
  if (message.t === "ping") post({ t: "pong", n: message.n });
  else if (message.t === "credit") {
    credits = Math.min(LIMITS.creditWindow, credits + 1);
    pump();
    drainTimers();
  } else if (message.t === "event" && pending.length < LIMITS.pendingBatches) {
    run(
      `__cowboyEvent(${literal(message.id)}, ${literal(message.name)}, ${literal(JSON.stringify(message.payload))})`,
      LIMITS.eventMs,
    );
    timersLeft = 1;
    drainTimers();
  } else if (message.t === "props") {
    run(`__cowboyProps(${literal(JSON.stringify(message.props))})`, LIMITS.eventMs);
  } else if (message.t === "context") {
    run(`__cowboyContext(${literal(JSON.stringify(message.context))})`, LIMITS.eventMs);
  } else if (message.t === "stop") {
    stop("closed");
  }
}

self.addEventListener(
  "message",
  (event) => {
    if (!event.ports?.[0] || !event.data) return;
    port = event.ports[0];
    port.onmessage = onHost;
    boot(event.data).catch((error) => stop(String(error?.message ?? error).slice(0, 500)));
  },
  { once: true },
);
