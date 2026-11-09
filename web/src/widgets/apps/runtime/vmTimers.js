// Runs inside the QuickJS VM before React loads (docs/widget-apps.md §5.1).
// The VM has no timers; React's scheduler captures `setTimeout` when its
// module initializes, so this shim must be the bundle's first module. The
// trusted Worker drains the queue under the VM's CPU budget.
globalThis.__cowboyTimers = [];
globalThis.setTimeout = (callback) => {
  globalThis.__cowboyTimers.push(callback);
  return globalThis.__cowboyTimers.length;
};
globalThis.clearTimeout = () => undefined;
globalThis.queueMicrotask = (callback) => {
  void Promise.resolve().then(callback);
};
