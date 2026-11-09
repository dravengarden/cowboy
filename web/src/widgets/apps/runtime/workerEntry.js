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
let ready = false;
let suspended = false;
let credits = LIMITS.creditWindow;
const pending = [];
let pendingBytes = 0;
const deferred = [];
let latestProps = null;
let latestContext = null;
let nextTimerWake = null;
let nextTimerAt = Infinity;
let jobsQueued = false;
const cpu = { spans: [], minuteStart: 0, minuteUsed: 0, throttled: 0 };
const rate = { sendAt: 0, sendDrops: 0, logStart: 0, logs: 0 };

// Shadow tree: parent and children of every node the app created.
const parentOf = new Map([[0, -1]]);
const childrenOf = new Map([[0, new Set()]]);
const depthOf = new Map([[0, 0]]);

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

function detach(id) {
  const parent = parentOf.get(id);
  if (parent !== undefined && parent >= 0) childrenOf.get(parent)?.delete(id);
  parentOf.set(id, -1);
}

function dropSubtree(id) {
  for (const child of childrenOf.get(id) ?? []) dropSubtree(child);
  detach(id);
  parentOf.delete(id);
  childrenOf.delete(id);
  depthOf.delete(id);
}

/** Attaches `child` under `parent` and re-derives the depth of its subtree. */
function attach(parent, child) {
  if (!childrenOf.has(parent) || !parentOf.has(child)) throw new Error("unknown node");
  for (let at = parent; at > 0; at = parentOf.get(at) ?? -1) {
    if (at === child) throw new Error("malformed render update");
  }
  detach(child);
  parentOf.set(child, parent);
  childrenOf.get(parent).add(child);
  const stack = [[child, (depthOf.get(parent) ?? 0) + 1]];
  while (stack.length > 0) {
    const [id, depth] = stack.pop();
    within(LIMITS.depth, depth, "the app nested too deeply");
    depthOf.set(id, depth);
    for (const next of childrenOf.get(id) ?? []) stack.push([next, depth + 1]);
  }
}

function create(id) {
  if (!parentOf.has(id)) {
    parentOf.set(id, -1);
    childrenOf.set(id, new Set());
    depthOf.set(id, 1);
  }
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
      create(id);
    } else if (kind === "text") {
      if (typeof op[2] !== "string") throw new Error("malformed text");
      within(LIMITS.textChars, op[2].length, "a text node was too long");
      create(id);
    } else if (kind === "append" || kind === "before") {
      attach(id, op[2]);
    } else if (kind === "remove") {
      dropSubtree(id);
    } else if (kind === "clear") {
      for (const child of [...(childrenOf.get(0) ?? [])]) dropSubtree(child);
    } else if (kind !== "props") {
      throw new Error("malformed render update");
    }
  }
  within(LIMITS.nodes, parentOf.size - 1, "the app rendered too many components");
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

/** CPU used by the VM in the last second (a rolling window). */
function usedLastSecond(now) {
  while (cpu.spans.length > 0 && cpu.spans[0].at <= now - 1000) cpu.spans.shift();
  return cpu.spans.reduce((sum, span) => sum + span.ms, 0);
}

/** Runs VM code under a deadline and the CPU budgets. `startup` calls
 *  (loading, compiling, first render) have their own deadlines and count
 *  toward the minute, not the rolling second. Returns `{ ok, value, busy }`;
 *  `busy` means the call did not run because the second is spent. */
function run(code, deadlineMs, startup = false) {
  const now = Date.now();
  if (now - cpu.minuteStart >= 60_000) {
    cpu.minuteStart = now;
    cpu.minuteUsed = 0;
  }
  const left = LIMITS.cpuPerSecondMs - usedLastSecond(now);
  if (!startup && left <= 0) {
    cpu.throttled += 1;
    if (cpu.throttled >= LIMITS.throttledTurns) stop("it kept the processor busy");
    return { ok: false, busy: true };
  }
  const until = now + (startup ? deadlineMs : Math.min(deadlineMs, left));
  runtime.setInterruptHandler(() => Date.now() > until);
  const result = vm.evalCode(code);
  let error = result.error ? vm.dump(result.error) : null;
  result.error?.dispose();
  const value = result.value ? vm.dump(result.value) : undefined;
  result.value?.dispose();
  let jobs = 0;
  while (!error && jobs < LIMITS.jobsPerTurn && runtime.hasPendingJob()) {
    const job = runtime.executePendingJobs(1);
    if (job.error) {
      error = vm.dump(job.error);
      job.error.dispose();
    }
    jobs += 1;
  }
  const spent = Date.now() - now;
  if (!startup) cpu.spans.push({ at: now, ms: spent });
  cpu.minuteUsed += spent;
  if (!startup && spent < left) cpu.throttled = 0;
  if (error) {
    fail(String(error?.message ?? error));
    return { ok: false };
  }
  if (cpu.minuteUsed > LIMITS.cpuPerMinuteMs) {
    stop("it kept the processor busy");
    return { ok: false };
  }
  if (runtime.hasPendingJob()) scheduleJobs(16);
  return { ok: true, value, jobs };
}

/** Continues promise jobs left over from a turn. A throttled turn retries;
 *  a suspended app or full draw queue waits for `resume` or a credit, which
 *  run timers and so the jobs too. Jobs can schedule timers, so the timer
 *  wake is recomputed afterwards. */
function scheduleJobs(delay) {
  if (jobsQueued) return;
  jobsQueued = true;
  setTimeout(() => {
    jobsQueued = false;
    if (stopped || suspended || !ready || pending.length >= LIMITS.pendingBatches) return;
    const result = run("undefined", LIMITS.eventMs);
    if (result.busy) return scheduleJobs(100);
    if (result.ok) wakeTimers(0);
  }, delay);
}

function hostFunctions() {
  const emit = vm.newFunction("__emit", (handle) => {
    try {
      admit(vm.getString(handle));
    } catch (error) {
      stop(String(error.message ?? error));
    }
  });
  // One message request per interval reaches the host; more are dropped,
  // and an app that keeps asking is stopped.
  const send = vm.newFunction("__send", (handle) => {
    const now = Date.now();
    const text = vm.getString(handle);
    if (text.length > LIMITS.messageChars * 2 || now - rate.sendAt < LIMITS.sendIntervalMs) {
      rate.sendDrops += 1;
      if (rate.sendDrops > LIMITS.sendDrops) stop("it asked to send messages too often");
      return;
    }
    rate.sendAt = now;
    post({ t: "send", text });
  });
  const log = vm.newFunction("__log", (handle) => {
    const now = Date.now();
    if (now - rate.logStart >= 1000) {
      rate.logStart = now;
      rate.logs = 0;
    }
    rate.logs += 1;
    if (rate.logs <= LIMITS.logsPerSecond)
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

/** Runs due timers and wakes again when the next one is due. */
function drainTimers() {
  nextTimerWake = null;
  nextTimerAt = Infinity;
  if (stopped || suspended || !ready) return;
  if (pending.length >= LIMITS.pendingBatches) return;
  const result = run(`__cowboyTimersRun(${LIMITS.timersPerTurn})`, LIMITS.eventMs);
  if (result.busy) return wakeTimers(100);
  if (!result.ok) return;
  // Promise jobs that ran after the timers may have scheduled new ones.
  if (result.jobs > 0) return wakeTimers(0);
  if (typeof result.value === "number" && result.value >= 0) wakeTimers(result.value);
}

/** Wakes the timer drain within `ms`; an earlier wake is never postponed,
 *  so a steady stream of job continuations cannot starve timers. */
function wakeTimers(ms) {
  const at = Date.now() + Math.max(16, ms);
  if (nextTimerWake !== null) {
    if (nextTimerAt <= at) return;
    clearTimeout(nextTimerWake);
  }
  nextTimerAt = at;
  nextTimerWake = setTimeout(drainTimers, at - Date.now());
}

/** Runs deferred host commands while the VM may run. */
function drainDeferred() {
  while (
    !stopped &&
    !suspended &&
    ready &&
    pending.length < LIMITS.pendingBatches &&
    deferred.length > 0
  ) {
    const command = deferred[0];
    const result = run(command.code, LIMITS.eventMs);
    if (result.busy) return setTimeout(drainDeferred, 100);
    deferred.shift();
  }
  wakeTimers(0);
}

function enqueue(kind, code) {
  if (kind !== "event") {
    // Newer props or context replace older ones still waiting.
    const index = deferred.findIndex((command) => command.kind === kind);
    if (index >= 0) deferred.splice(index, 1);
  }
  if (deferred.length >= LIMITS.deferredCommands) return stop("it fell too far behind");
  deferred.push({ kind, code });
  drainDeferred();
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
  if (!run(`__cowboyCompile(${literal(data.code)})`, LIMITS.compileMs, true).ok) return;
  const props = latestProps ?? JSON.stringify(data.props);
  const context = latestContext ?? JSON.stringify(data.context);
  if (!run(`__cowboyRender(${literal(props)}, ${literal(context)})`, LIMITS.firstRenderMs, true).ok)
    return;
  ready = true;
  drainDeferred();
}

function onHost(event) {
  const message = event.data;
  if (stopped || !message || typeof message !== "object") return;
  switch (message.t) {
    case "ping":
      post({ t: "pong", n: message.n });
      return;
    case "credit":
      credits = Math.min(LIMITS.creditWindow, credits + 1);
      pump();
      drainDeferred();
      return;
    case "suspend":
      suspended = true;
      return;
    case "resume":
      suspended = false;
      drainDeferred();
      return;
    case "event":
      enqueue(
        "event",
        `__cowboyEvent(${literal(message.id)}, ${literal(message.name)}, ${literal(JSON.stringify(message.payload))})`,
      );
      return;
    case "props":
      latestProps = JSON.stringify(message.props);
      if (ready) enqueue("props", `__cowboyProps(${literal(latestProps)})`);
      return;
    case "context":
      latestContext = JSON.stringify(message.context);
      if (ready) enqueue("context", `__cowboyContext(${literal(latestContext)})`);
      return;
    case "stop":
      stop("closed");
      return;
    default:
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
