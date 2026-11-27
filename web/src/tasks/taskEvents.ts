// Lightweight event & state bridge connecting DesktopWorkspace (Tasks tab),
// Transcript (anchors / origin tools), Composer (Mobile entry), approvals,
// shared call result quoting, and commands without circular imports.

import { useSyncExternalStore } from "react";
import type { TaskSummary } from "../backgroundTasks";
import { appendDraftText } from "../draftStore";
import { fetchCall, type ManagedCallDetail } from "../managedCalls";
import { requestStickToBottom } from "../stickyStore";
import { notify } from "../store";
import { formatTaskQuote } from "./taskTree";

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

// Per-worker, per-pane scroll position preservation (Result and Info kept separate).
const readerScrollByTask = new Map<string, number>();

export function rememberTaskReaderScroll(sessionId: string, scrollKey: string, top: number): void {
  readerScrollByTask.set(`${sessionId}\u0000${scrollKey}`, top);
  if (readerScrollByTask.size > 200) {
    const oldest = readerScrollByTask.keys().next().value;
    if (oldest !== undefined) readerScrollByTask.delete(oldest);
  }
}

export function savedTaskReaderScroll(sessionId: string, scrollKey: string): number {
  return readerScrollByTask.get(`${sessionId}\u0000${scrollKey}`) ?? 0;
}

// Shared identity-checked Call detail cache so Quote-to-Prompt includes loaded
// review/call results from every surface (Desktop header, Result body, Mobile dock).
const callDetails = new Map<string, ManagedCallDetail>();

export function rememberCallDetail(sessionId: string, detail: ManagedCallDetail): void {
  const key = `${sessionId}\u0000${detail.call_id}`;
  callDetails.set(key, detail);
  if (callDetails.size > 120) {
    const oldest = callDetails.keys().next().value;
    if (oldest !== undefined) callDetails.delete(oldest);
  }
}

export function cachedCallDetail(
  sessionId: string,
  callId: string,
  minUpdatedAtMs = 0,
): ManagedCallDetail | null {
  const detail = callDetails.get(`${sessionId}\u0000${callId}`) ?? null;
  if (!detail) return null;
  if (minUpdatedAtMs > 0 && detail.updated_at_ms < minUpdatedAtMs) return null;
  return detail;
}

export function callResultExcerpt(detail: ManagedCallDetail | null): string | null {
  if (!detail?.result) return null;
  if (detail.result.text?.trim()) return detail.result.text;
  if (detail.result.structured !== undefined && detail.result.structured !== null) {
    return JSON.stringify(detail.result.structured, null, 2);
  }
  return null;
}

/** Shared Quote-to-Prompt helper used by Mobile dock, Desktop header, and Result CTA.
 *  Runs synchronously when the call result is already cached (or for non-Calls),
 *  and fetches the exact session+call result when a completed Call has not loaded yet. */
export function quoteTaskToPrompt(
  sessionId: string,
  task: TaskSummary,
  onDone?: () => void,
): void {
  const finish = (excerpt: string | null): void => {
    appendDraftText(sessionId, formatTaskQuote(task, excerpt));
    notify("Quoted to Prompt", "info");
    onDone?.();
  };
  if (task.source !== "call" || !task.call) {
    finish(null);
    return;
  }
  const call = task.call;
  const cached = cachedCallDetail(sessionId, call.call_id, call.updated_at_ms);
  if (cached || !call.has_result) {
    finish(callResultExcerpt(cached));
    return;
  }
  const callId = call.call_id;
  void fetchCall(sessionId, callId)
    .then((detail) => {
      if (detail.call_id === callId) {
        rememberCallDetail(sessionId, detail);
        finish(callResultExcerpt(detail));
      } else {
        finish(null);
      }
    })
    .catch(() => {
      finish(null);
    });
}

function focusControlIn(surface: HTMLElement): boolean {
  surface.scrollIntoView({ block: "nearest" });
  const control = surface.querySelector<HTMLElement>(
    "button:not(:disabled), input:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])",
  );
  if (control) {
    control.focus({ preventScroll: true });
    return true;
  }
  return false;
}

/** Route `Review request` directly to the current session's live approval
 *  surface matching this task's request type and identity (never focusing an
 *  unrelated permission or another session's overlay). */
export function focusTaskApprovalSurface(sessionId: string, task: TaskSummary): boolean {
  const escSession = CSS.escape(sessionId);
  const escTarget = CSS.escape(task.key);
  const selectors = task.source === "approval"
    ? [
      `[data-desktop-approval-dock="call"][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
      `[data-call-approval-overlay][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
      `[data-desktop-approval-dock="call"][data-approval-session="${escSession}"]`,
      `[data-call-approval-overlay][data-approval-session="${escSession}"]`,
    ]
    : [
      `[data-desktop-approval-dock="permission"][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
      `[data-permission-overlay][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
    ];
  for (const selector of selectors) {
    const surface = document.querySelector<HTMLElement>(selector);
    if (!surface) continue;
    if (focusControlIn(surface)) return true;
    // Collapsed Mobile approval pill: expand it by sticking to bottom, then focus.
    requestStickToBottom(sessionId);
    requestAnimationFrame(() => {
      const expanded = document.querySelector<HTMLElement>(selector);
      if (expanded) focusControlIn(expanded);
    });
    return true;
  }
  notify("Answer the current session request above the composer first", "info");
  return false;
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
