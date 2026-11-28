// Product-neutral external Trigger sessions: pure classification, sorting,
// activity summary and outcome presentation shared by Mobile and Desktop.

import type { SessionMeta } from "./protocol";
import { isManagedChild } from "./managedCalls";
import type { FolderActivity } from "./sessionTree";
import { waitingOnBackground } from "./backgroundActivity";

export const TRIGGERS_ROOT_KEY = "triggers:root";

/**
 * True when a live session belongs to the dedicated Triggers section rather
 * than the user's primary folders/Global list. Covers both structured
 * `session.trigger` metadata and legacy `[External Intake:` runs created before
 * structured trigger metadata existed.
 */
export function isTriggerSession(session: SessionMeta): boolean {
  if (session.system || session.trashed_at_ms !== undefined || isManagedChild(session)) {
    return false;
  }
  if (session.trigger) return true;
  return (session.origin ?? "api") === "api" && session.title.startsWith("[External Intake:");
}

export function isAttentionTrigger(session: SessionMeta): boolean {
  const outcome = session.trigger?.outcome;
  if (outcome === "escalated" || outcome === "needs_approval" || outcome === "failed") {
    return true;
  }
  return session.status === "crashed" || session.status === "interrupted";
}

export function isSettledTrigger(session: SessionMeta): boolean {
  if (
    session.status === "busy" ||
    session.status === "starting" ||
    waitingOnBackground(session.status, session.background_tasks)
  ) {
    return false;
  }
  const outcome = session.trigger?.outcome;
  if (outcome === "running") return false;
  if (outcome === "resolved" || outcome === "suppressed") return true;
  return session.trigger === undefined && session.status !== "crashed";
}

export interface TriggerOutcomePresentation {
  readonly label: string;
  readonly color: "default" | "info" | "success" | "warning" | "error";
}

export interface TriggerDisplayMeta {
  readonly title: string;
  readonly source: string | null;
  readonly kind: string | null;
  readonly correlationKey: string | null;
}

const LEGACY_INTAKE_RE = /^\[External Intake:\s*([^\]]+)\]\s*(.*)$/;

/** Clean display title and source metadata for structured and legacy trigger runs. */
export function triggerDisplayMeta(session: SessionMeta): TriggerDisplayMeta {
  const rawTitle = (session.title || session.id).trim();
  const structuredSource = session.trigger?.source.trim() || null;
  const structuredKind = session.trigger?.kind.trim() || null;
  const correlationKey = session.trigger?.correlation_key?.trim() || null;

  if (structuredSource) {
    const prefix = `[${structuredSource}] `;
    const stripped = rawTitle.startsWith(prefix) ? rawTitle.slice(prefix.length).trim() : rawTitle;
    return {
      title: stripped || rawTitle,
      source: structuredSource,
      kind: structuredKind,
      correlationKey,
    };
  }

  const legacy = LEGACY_INTAKE_RE.exec(rawTitle);
  if (legacy) {
    const label = legacy[1]?.trim() || null;
    const rest = legacy[2]?.trim() || "";
    return {
      title: rest || label || rawTitle,
      source: label,
      kind: structuredKind ?? label,
      correlationKey,
    };
  }

  return {
    title: rawTitle,
    source: structuredKind,
    kind: structuredKind,
    correlationKey,
  };
}

/** Context window fill percentage (0..100), or null when unavailable/stale. */
export function triggerContextPercent(session: SessionMeta): number | null {
  if (session.usage && !session.usage.compacted_at_ms && session.usage.size > 0) {
    return Math.min(100, Math.round((session.usage.used / session.usage.size) * 100));
  }
  if (
    session.context_used !== undefined &&
    session.context_size !== undefined &&
    session.context_size > 0
  ) {
    return Math.min(100, Math.round((session.context_used / session.context_size) * 100));
  }
  return null;
}

/** Most recent observable timestamp for a trigger session, when present. */
export function triggerTimestampMs(session: SessionMeta): number | null {
  const settled = session.trigger?.settled_at_ms;
  const observed = session.usage?.observed_at_ms;
  if (settled !== undefined && observed !== undefined) {
    return Math.max(settled, observed);
  }
  return settled ?? observed ?? null;
}

export function formatTriggerAge(deltaMs: number): string {
  const mins = Math.floor(Math.max(0, deltaMs) / 60_000);
  if (mins < 1) return "now";
  if (mins < 60) return `${String(mins)}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${String(hours)}h`;
  return `${String(Math.floor(hours / 24))}d`;
}

export function triggerOutcomeMeta(session: SessionMeta): TriggerOutcomePresentation {
  if (session.status === "busy" || session.status === "starting") {
    return { label: "Running", color: "info" };
  }
  switch (session.trigger?.outcome) {
    case "running":
      return { label: "Running", color: "info" };
    case "resolved":
      return { label: "Resolved", color: "success" };
    case "suppressed":
      return { label: "Suppressed", color: "default" };
    case "needs_approval":
      return { label: "Needs approval", color: "warning" };
    case "escalated":
      return { label: "Escalated", color: "warning" };
    case "failed":
      return { label: "Failed", color: "error" };
    default:
      if (session.status === "crashed") return { label: "Failed", color: "error" };
      return { label: "Triggered", color: "default" };
  }
}

/**
 * Replace only the Trigger slots in `allDisplayed` with `reorderedTriggerIds`
 * so dragging inside Triggers persists order without disturbing ordinary
 * workspace sessions.
 */
export function mergeReorderedTriggers(
  allDisplayed: readonly SessionMeta[],
  reorderedTriggerIds: readonly string[],
): string[] {
  const triggerSet = new Set(reorderedTriggerIds);
  const queue = [...reorderedTriggerIds];
  let cursor = 0;
  const merged = allDisplayed.map((session) => {
    if (!triggerSet.has(session.id)) return session.id;
    const next = queue[cursor];
    cursor += 1;
    return next ?? session.id;
  });
  while (cursor < queue.length) {
    const rest = queue[cursor];
    if (rest !== undefined) merged.push(rest);
    cursor += 1;
  }
  return merged;
}

export interface TriggerSummary {
  readonly sessions: readonly SessionMeta[];
  readonly activity: FolderActivity;
  readonly settledIds: readonly string[];
}

export function summarizeTriggers(sessions: readonly SessionMeta[]): TriggerSummary {
  const list = sessions.filter(isTriggerSession);
  let working = 0;
  let attention = 0;
  let live = 0;
  const settledIds: string[] = [];
  for (const session of list) {
    if (
      session.status === "busy" ||
      session.status === "starting" ||
      waitingOnBackground(session.status, session.background_tasks)
    ) {
      working++;
    } else if (isAttentionTrigger(session)) {
      attention++;
    } else if (session.status === "running") {
      live++;
    }
    if (isSettledTrigger(session)) {
      settledIds.push(session.id);
    }
  }
  return {
    sessions: list,
    activity: { working, attention, live },
    settledIds,
  };
}
