// Desktop's jump list (FOCUS.md "Recent"): the Sessions and Drafts this
// device opened, most recent first. `␣⇥` returns to the previous one and `␣O`
// lists the rest. Keys are workspace item keys: a session id or
// `draft:<id>`. Per device and never synced; Mobile does not record it.

import { useSyncExternalStore } from "react";
import type { DraftMetadata } from "../documents/model";
import type { SessionMeta, Status } from "../protocol";
import { sessionProjectLabel } from "../sessionProject";

const STORAGE_KEY = "cowboy.desktop.visits.v1";
/** Remembered visits; deleted items fall out when the list is resolved. */
export const DESKTOP_VISIT_LIMIT = 30;
/** Rows in the Recent dialog: one per digit key. */
export const DESKTOP_RECENT_SHOWN = 9;

export interface DesktopVisit {
  readonly key: string;
  readonly at: number;
}

export interface DesktopRecentItem {
  readonly key: string;
  readonly kind: "session" | "draft";
  readonly title: string;
  readonly detail: string;
  readonly at: number;
  readonly provider?: string;
  readonly status?: Status;
}

/** Move `key` to the front, keeping the list short and free of duplicates. */
export function withDesktopVisit(
  visits: readonly DesktopVisit[],
  key: string,
  at: number,
): DesktopVisit[] {
  return [{ key, at }, ...visits.filter((visit) => visit.key !== key)]
    .slice(0, DESKTOP_VISIT_LIMIT);
}

/** The visits other than the current item that still exist, newest first. */
export function desktopRecentItems(
  visits: readonly DesktopVisit[],
  currentKey: string | null,
  sessions: readonly SessionMeta[],
  drafts: readonly DraftMetadata[],
  limit = DESKTOP_RECENT_SHOWN,
): DesktopRecentItem[] {
  const sessionById = new Map(sessions.map((session) => [session.id, session]));
  const draftById = new Map(
    drafts.filter((draft) => draft.kind === "document" && !draft.deleted)
      .map((draft) => [draft.id, draft]),
  );
  const items: DesktopRecentItem[] = [];
  for (const visit of visits) {
    if (items.length >= limit) break;
    if (visit.key === currentKey) continue;
    if (visit.key.startsWith("draft:")) {
      const draft = draftById.get(visit.key.slice("draft:".length));
      if (!draft) continue;
      items.push({
        key: visit.key,
        kind: "draft",
        title: draft.title || "Untitled draft",
        detail: "Draft",
        at: visit.at,
      });
      continue;
    }
    const session = sessionById.get(visit.key);
    if (!session) continue;
    items.push({
      key: visit.key,
      kind: "session",
      title: session.title,
      detail: sessionProjectLabel(session) ?? "",
      at: visit.at,
      provider: session.provider,
      status: session.status,
    });
  }
  return items;
}

function parse(raw: string | null): DesktopVisit[] {
  try {
    const value: unknown = JSON.parse(raw ?? "[]");
    if (!Array.isArray(value)) return [];
    return value.filter((visit): visit is DesktopVisit =>
      typeof visit === "object" && visit !== null &&
      typeof (visit as DesktopVisit).key === "string" &&
      typeof (visit as DesktopVisit).at === "number"
    ).slice(0, DESKTOP_VISIT_LIMIT);
  } catch {
    return [];
  }
}

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

let visits: readonly DesktopVisit[] = parse(
  storage()?.getItem(STORAGE_KEY) ?? null,
);
const listeners = new Set<() => void>();

export function recordDesktopVisit(key: string, at = Date.now()): void {
  if (visits[0]?.key === key) return;
  visits = withDesktopVisit(visits, key, at);
  try {
    storage()?.setItem(STORAGE_KEY, JSON.stringify(visits));
  } catch {
    // A full or blocked store only costs persistence across reloads.
  }
  for (const listener of listeners) listener();
}

export function useDesktopVisits(): readonly DesktopVisit[] {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => visits,
    () => visits,
  );
}
