// Lightweight event & state bridge connecting DesktopWorkspace (Tasks tab),
// Transcript (anchors / origin tools), Composer (Mobile entry), approvals,
// deep-link consumption, shared call result quoting, and commands.

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

export function publishSessionTasksAvailability(sessionId: string, next: SessionTasksAvailability): void {
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
      return (): void => { listeners.delete(listener); };
    },
    () => sessionTasksAvailability(sessionId),
    () => EMPTY_AVAILABILITY,
  );
}

// Tracks when DesktopWorkspace is hosting the non-modal Tasks work-pane view.
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
      return (): void => { listeners.delete(listener); };
    },
    () => (hostedDesktopSessions.get(sessionId) ?? 0) > 0,
    () => false,
  );
}

// Module-level deep-link consumption so closing a Mobile sheet after sibling
// switches or remounting a session-keyed Desktop view never reopens the link.
const consumedDeepLinksBySession = new Map<string, Set<string>>();

export function isDeepLinkConsumed(sessionId: string, rawHash: string): boolean {
  return consumedDeepLinksBySession.get(sessionId)?.has(rawHash) ?? false;
}

export function consumeDeepLink(sessionId: string, rawHash: string): void {
  let set = consumedDeepLinksBySession.get(sessionId);
  if (!set) {
    set = new Set();
    consumedDeepLinksBySession.set(sessionId, set);
  }
  set.add(rawHash);
}

export function clearConsumedDeepLinks(sessionId: string, keepHash?: string | null): void {
  const set = consumedDeepLinksBySession.get(sessionId);
  if (!set) return;
  if (!keepHash) {
    consumedDeepLinksBySession.delete(sessionId);
    return;
  }
  for (const hash of [...set]) {
    if (hash !== keepHash) set.delete(hash);
  }
  if (set.size === 0) consumedDeepLinksBySession.delete(sessionId);
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

export function callResultExcerpt(detail: Partial<Pick<ManagedCallDetail, "result">> | null | undefined): string | null {
  if (!detail?.result) return null;
  if (detail.result.text?.trim()) return detail.result.text;
  if (detail.result.structured !== undefined && detail.result.structured !== null) {
    return JSON.stringify(detail.result.structured, null, 2);
  }
  return null;
}

/** Shared Quote-to-Prompt helper used by Mobile dock, Desktop header, and Result CTA.
 *  Runs synchronously when the call result is already embedded/cached (or for non-Calls),
 *  and fetches the exact session+call detail unless `has_result === false`. */
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
  const embedded = callResultExcerpt(call as Partial<ManagedCallDetail>);
  if (embedded !== null) {
    finish(embedded);
    return;
  }
  const cached = cachedCallDetail(sessionId, call.call_id, call.updated_at_ms);
  if (cached || call.has_result === false) {
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

/** Route `Review request` strictly to the current session's live approval
 *  surface matching this task's exact request type and presented identity. */
export function focusTaskApprovalSurface(sessionId: string, task: TaskSummary): boolean {
  const escSession = CSS.escape(sessionId);
  const escTarget = CSS.escape(task.key);
  const selectors = task.source === "approval"
    ? [
      `[data-desktop-approval-dock="call"][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
      `[data-call-approval-overlay][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
    ]
    : [
      `[data-desktop-approval-dock="permission"][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
      `[data-permission-overlay][data-approval-session="${escSession}"][data-approval-target="${escTarget}"]`,
    ];
  for (const selector of selectors) {
    const surface = document.querySelector<HTMLElement>(selector);
    if (!surface) continue;
    if (focusControlIn(surface)) return true;
    // Collapsed Mobile approval pill: expand it by sticking to bottom, then verify the exact match still exists.
    requestStickToBottom(sessionId);
    requestAnimationFrame(() => {
      const expanded = document.querySelector<HTMLElement>(selector);
      if (expanded && focusControlIn(expanded)) return;
      notify("This approval request has changed or is no longer active.", "info");
    });
    return true;
  }
  notify("This approval request has changed or is no longer active.", "info");
  return false;
}

// Session-scoped pending Locate / Origin Tool reveals so switching from Explore
// to History delivers the navigation as soon as Transcript mounts.
export const LOCATE_TASK_EVENT = "cowboy:locate-background-task";
export const OPEN_TASK_ORIGIN_TOOL_EVENT = "cowboy:open-task-origin-tool";

const pendingLocateBySession = new Map<string, string>();
const pendingOriginToolBySession = new Map<string, string>();

export function dispatchLocateTask(sessionId: string, key: string): void {
  pendingLocateBySession.set(sessionId, key);
  globalThis.dispatchEvent(
    new CustomEvent<{ sessionId: string; key: string }>(LOCATE_TASK_EVENT, {
      detail: { sessionId, key },
    }),
  );
}

export function consumePendingLocateTask(sessionId: string): string | null {
  const key = pendingLocateBySession.get(sessionId) ?? null;
  if (key !== null) pendingLocateBySession.delete(sessionId);
  return key;
}

export function dispatchOpenTaskOriginTool(sessionId: string, toolKey: string): void {
  pendingOriginToolBySession.set(sessionId, toolKey);
  globalThis.dispatchEvent(
    new CustomEvent<{ sessionId: string; toolKey: string }>(OPEN_TASK_ORIGIN_TOOL_EVENT, {
      detail: { sessionId, toolKey },
    }),
  );
}

export function consumePendingOpenTaskOriginTool(sessionId: string): string | null {
  const key = pendingOriginToolBySession.get(sessionId) ?? null;
  if (key !== null) pendingOriginToolBySession.delete(sessionId);
  return key;
}
