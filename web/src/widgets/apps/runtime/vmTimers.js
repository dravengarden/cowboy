// Runs inside the QuickJS VM before React loads (docs/widget-apps.md §5.1).
// The VM has no timers; React's scheduler captures `setTimeout` when its
// module initializes, so this shim must be the bundle's first module. Timers
// keep their delay and can be cancelled; the trusted Worker runs the due ones
// under the VM's CPU budget and wakes again when the next one is due.
const timers = new Map();
let nextTimer = 1;
const schedule = (callback, delay, args, every) => {
  const id = nextTimer++;
  const wait = Math.max(0, Number(delay) || 0);
  timers.set(id, { callback, args, due: Date.now() + wait, every: every ? Math.max(16, wait) : 0 });
  return id;
};
globalThis.setTimeout = (callback, delay, ...args) => schedule(callback, delay, args, false);
globalThis.setInterval = (callback, delay, ...args) => schedule(callback, delay, args, true);
globalThis.clearTimeout = (id) => void timers.delete(id);
globalThis.clearInterval = (id) => void timers.delete(id);
globalThis.queueMicrotask = (callback) => {
  void Promise.resolve().then(callback);
};
/** Runs up to `max` due timers; returns ms until the next one, or -1. */
globalThis.__cowboyTimersDue = (max) => {
  const now = Date.now();
  const due = [...timers]
    .filter(([, timer]) => timer.due <= now)
    .sort((a, b) => a[1].due - b[1].due);
  for (const [id, timer] of due.slice(0, max)) {
    if (timer.every) timer.due = now + timer.every;
    else timers.delete(id);
    if (typeof timer.callback === "function") timer.callback(...timer.args);
  }
  let next = -1;
  for (const timer of timers.values()) {
    const wait = Math.max(0, timer.due - Date.now());
    if (next < 0 || wait < next) next = wait;
  }
  return next;
};
