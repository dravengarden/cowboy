// Lightweight event & state bridge connecting DesktopWorkspace (Tasks tab),
// Transcript (anchors / origin tools), Composer (Mobile entry) and commands
// without circular imports or hidden DOM markers.

import { useSyncExternalStore } from "react";

export interface SessionTasksAvailability {
  hasTasks: boolean;
  activeCount: number;
  needsYouCount: number;
  failedUnseenCount: number;
}

const EMPTY_AVAILABILITY: SessionTasksAvailability = {
  hasTasks: false,
  activeCount: 0,
  needsYouCount: 0,
  failedUnseenCount: 0,
};

let activeSessionId: string | null = null;
const availabilityBySession = new Map<string, SessionTasksAvailability>();
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

export function publishSessionTasksAvailability(
  sessionId: string,
  next: SessionTasksAvailability,
): void {
  const prev = availabilityBySession.get(sessionId);
  activeSessionId = sessionId;
  if (
    prev &&
    prev.hasTasks === next.hasTasks &&
    prev.activeCount === next.activeCount &&
    prev.needsYouCount === next.needsYouCount &&
    prev.failedUnseenCount === next.failedUnseenCount
  ) return;
  availabilityBySession.set(sessionId, next);
  emit();
}

export function clearSessionTasksAvailability(sessionId: string): void {
  if (activeSessionId === sessionId) activeSessionId = null;
  if (availabilityBySession.delete(sessionId)) emit();
}

export function sessionTasksAvailability(sessionId?: string | null): SessionTasksAvailability {
  const id = sessionId ?? activeSessionId;
  if (!id) return EMPTY_AVAILABILITY;
  return availabilityBySession.get(id) ?? EMPTY_AVAILABILITY;
}

export function activeTasksSessionId(): string | null {
  return activeSessionId;
}

export function useSessionTasksAvailability(sessionId: string | null): SessionTasksAvailability {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return (): void => {
        listeners.delete(listener);
      };
    },
    () => sessionTasksAvailability(sessionId),
    () => EMPTY_AVAILABILITY,
  );
}

// Tracks when DesktopWorkspace is hosting the non-modal Tasks work-pane view
// for a session, so Transcript's embedded BackgroundTasksInspector stands down
// on Desktop while still powering standalone fixtures and Mobile.
const hostedDesktopSessions = new Map<string, number>();

export function registerDesktopTasksHost(sessionId: string): () => void {
  hostedDesktopSessions.set(sessionId, (hostedDesktopSessions.get(sessionId) ?? 0) + 1);
  emit();
  return () => {
    const remaining = (hostedDesktopSessions.get(sessionId) ?? 1) - 1;
    if (remaining <= 0) hostedDesktopSessions.delete(sessionId);
    else hostedDesktopSessions.set(sessionId, remaining);
    emit();
  };
}

export function useDesktopTasksHostActive(sessionId: string): boolean {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return (): void => {
        listeners.delete(listener);
      };
    },
    () => (hostedDesktopSessions.get(sessionId) ?? 0) > 0,
    () => false,
  );
}

// Per-worker scroll position preservation across sibling switches.
const readerScrollByTask = new Map<string, number>();

export function rememberTaskReaderScroll(sessionId: string, taskKey: string, top: number): void {
  readerScrollByTask.set(`${sessionId}\u0000${taskKey}`, top);
  if (readerScrollByTask.size > 120) {
    const oldest = readerScrollByTask.keys().next().value;
    if (oldest !== undefined) readerScrollByTask.delete(oldest);
  }
}

export function savedTaskReaderScroll(sessionId: string, taskKey: string): number {
  return readerScrollByTask.get(`${sessionId}\u0000${taskKey}`) ?? 0;
}

/** Ask the mounted Transcript to scroll to a task's anchor row. */
export const LOCATE_TASK_EVENT = "cowboy:locate-background-task";

export function dispatchLocateTask(sessionId: string, key: string): void {
  globalThis.dispatchEvent(
    new CustomEvent<{ sessionId: string; key: string }>(LOCATE_TASK_EVENT, {
      detail: { sessionId, key },
    }),
  );
}

/** Ask the mounted Transcript to open an MCP Call's originating tool card. */
export const OPEN_TASK_ORIGIN_TOOL_EVENT = "cowboy:open-task-origin-tool";

export function dispatchOpenTaskOriginTool(sessionId: string, toolKey: string): void {
  globalThis.dispatchEvent(
    new CustomEvent<{ sessionId: string; toolKey: string }>(OPEN_TASK_ORIGIN_TOOL_EVENT, {
      detail: { sessionId, toolKey },
    }),
  );
}
