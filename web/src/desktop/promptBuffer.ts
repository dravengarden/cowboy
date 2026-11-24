import { useSyncExternalStore } from "react";
import { stripImageTokens } from "../attachments";
import type { QueueEditHoldStatus } from "../queueEditingHold";
import type { QueuedMessage } from "../store";

export type DesktopPromptBuffer =
  | { kind: "new" }
  | {
    kind: "queued" | "draft";
    id: string;
    cmid?: string;
    origin: "list" | "new";
    entryMode: "insert" | "normal";
  }
  | {
    kind: "new-draft";
    origin: "list" | "new";
    entryMode: "insert" | "normal";
  };

export interface LastSubmittedPending {
  kind: "queued" | "draft";
  cmid?: string;
  textHint: string;
  at: number;
}

export interface DesktopPromptStatus {
  kind: "new" | "draft" | "scheduled" | "queued" | "new-draft" | "document";
  index?: number;
  total?: number;
  dirty: boolean;
  holdStatus: QueueEditHoldStatus;
  scheduleLabel?: string;
  sendLabel: string;
  canForce: boolean;
}

export const DESKTOP_TOGGLE_EMPTY_DRAFTS_EVENT = "cowboy:desktop-toggle-empty-drafts";
export const DESKTOP_NEW_SESSION_DRAFT_EVENT = "cowboy:desktop-new-session-draft";
export const DESKTOP_PENDING_ROW_EDIT_EVENT = "cowboy:desktop-pending-row-edit";
export const DESKTOP_OPEN_SESSION_DRAFT_EVENT = "cowboy:desktop-open-session-draft";

const pendingOpenDraftBySession = new Map<string, { id?: string; cmid?: string }>();

/** Queue a session draft to open in the Desktop Prompt canvas as soon as that
 *  Session's workbench is mounted (and also notify if already mounted). */
export function requestOpenSessionDraft(
  sessionId: string,
  target: { id?: string; cmid?: string },
): void {
  pendingOpenDraftBySession.set(sessionId, target);
  globalThis.dispatchEvent(
    new CustomEvent(DESKTOP_OPEN_SESSION_DRAFT_EVENT, {
      detail: { sessionId, ...target },
    }),
  );
}

export function consumePendingOpenSessionDraft(
  sessionId: string,
): { id?: string; cmid?: string } | null {
  const next = pendingOpenDraftBySession.get(sessionId) ?? null;
  if (next) pendingOpenDraftBySession.delete(sessionId);
  return next;
}

const DEFAULT_PROMPT_STATUS: DesktopPromptStatus = {
  kind: "new",
  dirty: false,
  holdStatus: "none",
  sendLabel: "Send",
  canForce: false,
};

let currentPromptStatus: DesktopPromptStatus = DEFAULT_PROMPT_STATUS;
const statusListeners = new Set<() => void>();

export function setDesktopPromptStatus(next: DesktopPromptStatus): void {
  const prev = currentPromptStatus;
  if (
    prev.kind === next.kind &&
    prev.index === next.index &&
    prev.total === next.total &&
    prev.dirty === next.dirty &&
    prev.holdStatus === next.holdStatus &&
    prev.scheduleLabel === next.scheduleLabel &&
    prev.sendLabel === next.sendLabel &&
    prev.canForce === next.canForce
  ) return;
  currentPromptStatus = next;
  for (const listener of statusListeners) listener();
}

export function resetDesktopPromptStatus(): void {
  setDesktopPromptStatus(DEFAULT_PROMPT_STATUS);
}

export function useDesktopPromptStatus(): DesktopPromptStatus {
  return useSyncExternalStore(
    (cb) => {
      statusListeners.add(cb);
      return () => statusListeners.delete(cb);
    },
    () => currentPromptStatus,
    () => DEFAULT_PROMPT_STATUS,
  );
}

const lastSubmittedBySession = new Map<string, LastSubmittedPending>();

export function recordLastSubmittedPending(
  sessionId: string,
  entry: LastSubmittedPending | null,
): void {
  if (entry === null) lastSubmittedBySession.delete(sessionId);
  else lastSubmittedBySession.set(sessionId, entry);
}

export function getLastSubmittedPending(
  sessionId: string,
): LastSubmittedPending | null {
  return lastSubmittedBySession.get(sessionId) ?? null;
}

/** Pending buffer identity: match by server id first, then cmid fallback so
 *  an optimistic row confirming into a server id stays seamlessly bound. */
export function resolveBufferRow(
  buffer: DesktopPromptBuffer,
  queue: readonly QueuedMessage[],
  drafts: readonly QueuedMessage[],
): { row: QueuedMessage; index: number; total: number } | null {
  if (buffer.kind !== "queued" && buffer.kind !== "draft") return null;
  const rows = buffer.kind === "queued" ? queue : drafts;
  let index = rows.findIndex((item) => item.id === buffer.id);
  if (index < 0 && buffer.cmid) {
    index = rows.findIndex((item) => item.cmid === buffer.cmid);
  }
  if (index < 0) return null;
  const row = rows[index];
  return row ? { row, index, total: rows.length } : null;
}

/** Resolve the last queued/parked message from this Prompt for `↑` recall or
 *  `␣Q` smart cursor selection. Skips call-notice / current-turn rows. */
export function resolveLastSubmittedRow(
  last: LastSubmittedPending | null,
  queue: readonly QueuedMessage[],
  drafts: readonly QueuedMessage[],
): { kind: "queued" | "draft"; row: QueuedMessage } | null {
  if (!last) return null;
  const rows = (last.kind === "queued" ? queue : drafts).filter(
    (item) => !item.currentTurn && item.status === undefined,
  );
  if (rows.length === 0) return null;
  if (last.cmid) {
    const byCmid = rows.find((item) => item.cmid === last.cmid);
    if (byCmid) return { kind: last.kind, row: byCmid };
  }
  const byText = [...rows].reverse().find(
    (item) => item.text.trimEnd() === last.textHint.trimEnd(),
  );
  if (byText) return { kind: last.kind, row: byText };
  return null;
}

/** Walk along send order (`[q` / `]q`): editable Queue rows in order, with New
 *  Prompt at the tail. Returns null when already at the requested end. */
export function adjacentSendOrderBuffer(
  current: DesktopPromptBuffer,
  queue: readonly QueuedMessage[],
  delta: -1 | 1,
): DesktopPromptBuffer | null {
  const editable = queue.filter((item) => !item.currentTurn && item.status === undefined);
  if (editable.length === 0) return null;
  const slots: DesktopPromptBuffer[] = [
    ...editable.map((item): DesktopPromptBuffer => ({
      kind: "queued",
      id: item.id,
      ...(item.cmid ? { cmid: item.cmid } : {}),
      origin: "new",
      entryMode: "normal",
    })),
    { kind: "new" },
  ];
  let idx = slots.length - 1;
  if (current.kind === "queued") {
    const found = editable.findIndex(
      (item) => item.id === current.id || (current.cmid !== undefined && item.cmid === current.cmid),
    );
    if (found >= 0) idx = found;
  } else if (current.kind !== "new") {
    return delta < 0 ? slots[editable.length - 1] ?? null : { kind: "new" };
  }
  const next = idx + delta;
  if (next < 0 || next >= slots.length) return null;
  return slots[next] ?? null;
}

/** Single-line glanceable preview for persistent Desktop Queue/Draft headers. */
export function firstLinePreview(text: string, maxChars = 72): string {
  const cleaned = stripImageTokens(text)
    .replace(/\r?\n+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return "(attachment)";
  if (cleaned.length <= maxChars) return cleaned;
  return `${cleaned.slice(0, maxChars - 1).trimEnd()}…`;
}
