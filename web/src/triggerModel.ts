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

function triggerRank(session: SessionMeta): number {
  if (
    session.status === "busy" ||
    session.status === "starting" ||
    session.trigger?.outcome === "running"
  ) {
    return 0;
  }
  if (isAttentionTrigger(session)) return 1;
  return 2;
}

export interface TriggerSummary {
  readonly sessions: readonly SessionMeta[];
  readonly activity: FolderActivity;
  readonly settledIds: readonly string[];
}

export function summarizeTriggers(sessions: readonly SessionMeta[]): TriggerSummary {
  const list = sessions.filter(isTriggerSession);
  list.sort((a, b) => triggerRank(a) - triggerRank(b));
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
