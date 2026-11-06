import { useCallback, useSyncExternalStore } from "react";
import { type CodeChecks, fetchCodeChecks } from "./codeApi";

/** While a check runs, its result is worth seeing promptly. */
const RUNNING_POLL_MS = 3_000;
const IDLE_POLL_MS = 20_000;
/** An older Controller has no checks route; ask again rarely. */
const FAILED_POLL_MS = 60_000;

interface Entry {
  data: CodeChecks | undefined;
  listeners: Set<() => void>;
  timer: ReturnType<typeof setTimeout> | undefined;
  controller: AbortController | undefined;
}

const entries = new Map<string, Entry>();

async function refresh(sessionId: string, entry: Entry): Promise<void> {
  if (entry.listeners.size === 0) return;
  let delay = IDLE_POLL_MS;
  if (globalThis.document?.visibilityState !== "hidden") {
    const controller = new AbortController();
    entry.controller = controller;
    try {
      const data = await fetchCodeChecks(sessionId, controller.signal);
      if (controller.signal.aborted) return;
      entry.data = data;
      if (data.runs.some((run) => run.state === "running")) {
        delay = RUNNING_POLL_MS;
      }
      for (const listener of entry.listeners) listener();
    } catch {
      if (controller.signal.aborted) return;
      delay = FAILED_POLL_MS;
    }
  }
  if (entry.listeners.size === 0) return;
  entry.timer = setTimeout(() => void refresh(sessionId, entry), delay);
}

/** Every Review surface of a session shares one poll of its check results. */
function subscribe(sessionId: string, listener: () => void): () => void {
  let entry = entries.get(sessionId);
  if (!entry) {
    entry = {
      data: undefined,
      listeners: new Set(),
      timer: undefined,
      controller: undefined,
    };
    entries.set(sessionId, entry);
  }
  const current = entry;
  current.listeners.add(listener);
  if (current.listeners.size === 1) void refresh(sessionId, current);
  return () => {
    current.listeners.delete(listener);
    if (current.listeners.size > 0) return;
    clearTimeout(current.timer);
    current.controller?.abort();
    entries.delete(sessionId);
  };
}

export function useReviewChecks(sessionId: string): CodeChecks | undefined {
  const subscribeSession = useCallback(
    (listener: () => void) => subscribe(sessionId, listener),
    [sessionId],
  );
  return useSyncExternalStore(
    subscribeSession,
    () => entries.get(sessionId)?.data,
  );
}

export function refreshReviewChecks(sessionId: string): void {
  const entry = entries.get(sessionId);
  if (!entry) return;
  clearTimeout(entry.timer);
  entry.controller?.abort();
  void refresh(sessionId, entry);
}
