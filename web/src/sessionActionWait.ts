type Pending = { label: string; phase: "waiting" | "sending"; cancel: () => void };
export class SessionActionCancelled extends Error {
  constructor() { super("Action cancelled; nothing was changed."); this.name = "SessionActionCancelled"; }
}
let pending: Pending | null = null;
const listeners = new Set<() => void>();
export function waitForSnapshot<T>(read: () => T, observers: Set<() => void>, predicate: (value: T) => boolean, label: string, timeoutMs: number): Promise<void> {
  if (predicate(read())) return Promise.resolve();
  return new Promise((resolve, reject) => {
    let timeout = 0;
    const check = (): void => {
      if (!predicate(read())) return;
      observers.delete(check);
      globalThis.clearTimeout(timeout);
      resolve();
    };
    observers.add(check);
    timeout = globalThis.setTimeout(() => {
      observers.delete(check);
      reject(new Error(`${label} was not acknowledged`));
    }, timeoutMs);
    check();
  });
}
export const sessionActionSnapshot = (): Pending | null => pending;
export function subscribeSessionAction(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
function publish(value: Pending | null): void {
  pending = value;
  for (const listener of listeners) listener();
}

/** Only unsent intent can be cancelled. Never replay a command after send. */
export async function waitForSessionAction(options: {
  label: string;
  ready: () => boolean;
  reconnect: () => void;
  unchanged: () => boolean;
  execute: () => Promise<void>;
  prepare?: () => Promise<void>;
  timeoutMs?: number;
}): Promise<void> {
  if (pending) throw new Error("Finish or cancel the pending session action first.");
  let cancelled = false;
  let rejectCancellation!: (error: Error) => void;
  const cancellation = new Promise<never>((_, reject) => { rejectCancellation = reject; });
  void cancellation.catch(() => {});
  const cancel = (): void => { cancelled = true; rejectCancellation(new SessionActionCancelled()); };
  const deadline = Date.now() + (options.timeoutMs ?? 60_000);
  publish({ label: options.label, phase: "waiting", cancel });
  try {
    options.reconnect();
    while (!options.ready()) {
      if (cancelled) throw new SessionActionCancelled();
      if (Date.now() >= deadline) throw new Error("Still unable to connect. Nothing was changed; please try again when connected.");
      await Promise.race([cancellation, new Promise((resolve) => globalThis.setTimeout(resolve, 100))]);
    }
    if (cancelled) throw new SessionActionCancelled();
    await Promise.race([cancellation, options.prepare?.()]);
    if (cancelled) throw new SessionActionCancelled();
    if (Date.now() >= deadline) throw new Error("Connection check expired. Nothing was changed; please try again.");
    if (!options.ready()) throw new Error("Connection changed. Nothing was changed; please try again.");
    if (!options.unchanged()) throw new Error("The conversation changed while reconnecting. Nothing was cleared; review it before trying again.");
    publish({ label: options.label, phase: "sending", cancel });
    await options.execute();
  } finally {
    publish(null);
  }
}
