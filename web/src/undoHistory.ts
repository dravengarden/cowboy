// Undo history: every undoable change this device made, for UNDO_TTL_MS.
// The snackbar offers the newest entry for UNDO_NOTICE_MS; the history sheet
// keeps the rest reachable after it disappears. Entries hold serializable
// steps (never closures), so an undo survives a reload and another tab. The
// executors live in undoActions.ts; this module is the pure model and the
// device-local store. See docs/undo-history.md.
import type { Attachment } from "./attachments";
import { PRODUCT_SESSION_END_EVENT } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";

/** Every transient notice, undoable or not, shows for the same time. */
export const UNDO_NOTICE_MS = 3_000;
/** Undo is for recent mistakes; Trash keeps deleted items far longer. */
export const UNDO_TTL_MS = 24 * 60 * 60 * 1000;
export const UNDO_MAX_ENTRIES = 100;
/** Entries whose steps exceed this keep them only in memory. */
const PERSISTED_STEPS_LIMIT = 256 * 1024;
const STORAGE_KEY = "cowboy.undoHistory.v1";

export type UndoStep =
  | { kind: "restore_trash"; sessions?: string[]; folders?: string[] }
  | { kind: "restore_draft"; id: string }
  | { kind: "trash_draft"; id: string; revision: number }
  | {
    kind: "remove_session_draft";
    sessionId: string;
    cmid: string;
    text: string;
    attachments: Attachment[];
  }
  | {
    kind: "copy_to_session";
    sessionId: string;
    text: string;
    attachments: Attachment[];
  }
  | { kind: "restore_calls"; parent: string; ids: string[] };

export type UndoState = "available" | "undoing" | "used" | "unavailable";

export interface UndoEntry {
  id: string;
  message: string;
  createdAt: number;
  principal: string | null;
  /** Null when the steps were too large to keep across a reload. */
  steps: UndoStep[] | null;
  state: UndoState;
  /** When the entry was undone or became unavailable. */
  settledAt?: number;
  reason?: string;
}

// ---- pure model -----------------------------------------------------------

export function pruneUndoEntries(
  entries: readonly UndoEntry[],
  now: number,
): UndoEntry[] {
  return entries
    .filter((entry) => now - entry.createdAt < UNDO_TTL_MS)
    .sort((a, b) => b.createdAt - a.createdAt)
    .slice(0, UNDO_MAX_ENTRIES);
}

/** Reload view: an undo interrupted by closing the page has an unknown
 *  outcome, so it is never offered again. */
export function reviveUndoEntry(entry: UndoEntry): UndoEntry {
  return entry.state === "undoing"
    ? {
      ...entry,
      state: "unavailable",
      reason: "The page closed while undoing. Check the item before retrying.",
    }
    : entry;
}

/** Displayed state: an entry whose steps stayed in another page's memory
 *  cannot run here. Derived, so this page never overwrites that page. */
export function effectiveUndoState(
  entry: UndoEntry,
): { state: UndoState; reason?: string } {
  if (entry.state === "available" && entry.steps === null) {
    return {
      state: "unavailable",
      reason: "Too large to keep outside the page that made this change.",
    };
  }
  return entry.reason === undefined
    ? { state: entry.state }
    : { state: entry.state, reason: entry.reason };
}

const STEP_KINDS = new Set([
  "restore_trash",
  "restore_draft",
  "trash_draft",
  "remove_session_draft",
  "copy_to_session",
  "restore_calls",
]);
const STATES = new Set(["available", "undoing", "used", "unavailable"]);

function validEntry(value: unknown): value is UndoEntry {
  if (!value || typeof value !== "object") return false;
  const entry = value as Partial<UndoEntry>;
  return typeof entry.id === "string" && typeof entry.message === "string" &&
    typeof entry.createdAt === "number" && Number.isFinite(entry.createdAt) &&
    (entry.principal === null || typeof entry.principal === "string") &&
    STATES.has(entry.state as string) &&
    (entry.steps === null ||
      (Array.isArray(entry.steps) &&
        entry.steps.every((step) =>
          step && typeof step === "object" &&
          STEP_KINDS.has((step as { kind?: string }).kind ?? "")
        )));
}

export function parseUndoEntries(raw: string | null): UndoEntry[] {
  if (!raw) return [];
  try {
    const value: unknown = JSON.parse(raw);
    return Array.isArray(value) ? value.filter(validEntry) : [];
  } catch {
    return [];
  }
}

/** Storage form: oversized steps stay in memory only. */
export function serializeUndoEntries(entries: readonly UndoEntry[]): string {
  return JSON.stringify(entries.map((entry) =>
    entry.steps && JSON.stringify(entry.steps).length > PERSISTED_STEPS_LIMIT
      ? { ...entry, steps: null }
      : entry
  ));
}

export interface TrashView {
  /** Both lists reflect the Controller (not a cold boot). */
  ready: boolean;
  liveSessions: ReadonlySet<string>;
  trashedSessions: ReadonlySet<string>;
  /** Folder id → whether it is in Trash; absent means purged. */
  folders: ReadonlyMap<string, boolean>;
}

/** Why a Trash restore can no longer run, from the live Trash view: the item
 *  came back some other way, or Trash deleted it for good. */
export function trashUndoBlocker(
  steps: readonly UndoStep[] | null,
  view: TrashView,
): string | undefined {
  if (!view.ready || !steps) return undefined;
  for (const step of steps) {
    if (step.kind !== "restore_trash") continue;
    for (const id of step.sessions ?? []) {
      if (view.trashedSessions.has(id)) continue;
      return view.liveSessions.has(id)
        ? "Already restored from Trash."
        : "No longer in Trash: deleted permanently.";
    }
    for (const id of step.folders ?? []) {
      const trashed = view.folders.get(id);
      if (trashed === true) continue;
      return trashed === false
        ? "Already restored from Trash."
        : "No longer in Trash: deleted permanently.";
    }
  }
  return undefined;
}

// ---- device-local store ---------------------------------------------------

let entries: UndoEntry[] = [];
let loaded = false;
const memorySteps = new Map<string, UndoStep[]>();
const listeners = new Set<() => void>();
let snapshot: readonly UndoEntry[] = [];
let snapshotOwner: string | null | undefined;

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

function principal(): string | null {
  return productSyncPrincipal() ?? null;
}

function project(): void {
  snapshotOwner = principal();
  snapshot = entries.filter((entry) => entry.principal === snapshotOwner);
}

function publish(): void {
  project();
  for (const listener of listeners) listener();
}

function persist(): void {
  try {
    storage()?.setItem(STORAGE_KEY, serializeUndoEntries(entries));
  } catch {
    // A full or blocked store keeps this tab's history in memory only.
  }
}

function load(): void {
  const stored = parseUndoEntries(storage()?.getItem(STORAGE_KEY) ?? null);
  entries = pruneUndoEntries(
    stored.map((entry) => {
      const steps = entry.steps ?? memorySteps.get(entry.id) ?? null;
      return reviveUndoEntry({ ...entry, steps });
    }),
    Date.now(),
  );
}

function ensureLoaded(): void {
  if (loaded) return;
  loaded = true;
  load();
  globalThis.addEventListener?.("storage", (event: StorageEvent) => {
    if (event.key !== STORAGE_KEY) return;
    entries = pruneUndoEntries(
      parseUndoEntries(event.newValue).map((entry) => ({
        ...entry,
        steps: entry.steps ?? memorySteps.get(entry.id) ?? null,
      })),
      Date.now(),
    );
    publish();
  });
  // Entries may quote Draft text: sign-out removes them with the session.
  globalThis.addEventListener?.(PRODUCT_SESSION_END_EVENT, () => {
    entries = [];
    memorySteps.clear();
    try {
      storage()?.removeItem(STORAGE_KEY);
    } catch {
      // Nothing else to clear.
    }
    publish();
  });
  globalThis.setInterval?.(pruneUndoHistory, 10 * 60 * 1000);
  publish();
}

function write(next: UndoEntry[]): void {
  entries = pruneUndoEntries(next, Date.now());
  for (const id of memorySteps.keys()) {
    if (!entries.some((entry) => entry.id === id)) memorySteps.delete(id);
  }
  persist();
  publish();
}

export function pruneUndoHistory(): void {
  ensureLoaded();
  const kept = pruneUndoEntries(entries, Date.now());
  if (kept.length !== entries.length) write(kept);
}

export function addUndoEntry(message: string, steps: UndoStep[]): UndoEntry {
  ensureLoaded();
  const entry: UndoEntry = {
    id: globalThis.crypto?.randomUUID?.() ??
      `${String(Date.now())}-${String(Math.random()).slice(2)}`,
    message,
    createdAt: Date.now(),
    principal: principal(),
    steps,
    state: "available",
  };
  memorySteps.set(entry.id, steps);
  write([entry, ...entries]);
  return entry;
}

export function updateUndoEntry(
  id: string,
  change: Pick<UndoEntry, "state"> & Partial<Pick<UndoEntry, "reason">>,
): void {
  ensureLoaded();
  write(entries.map((entry) =>
    entry.id === id
      ? {
        ...entry,
        state: change.state,
        ...(change.state === "undoing" || change.state === "available"
          ? {}
          : { settledAt: Date.now() }),
        ...(change.reason === undefined ? {} : { reason: change.reason }),
      }
      : entry
  ));
}

export function undoEntryById(id: string): UndoEntry | undefined {
  ensureLoaded();
  return entries.find((entry) => entry.id === id);
}

export function subscribeUndoHistory(listener: () => void): () => void {
  ensureLoaded();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function readUndoHistory(): readonly UndoEntry[] {
  ensureLoaded();
  // The principal binds after boot; re-project once it does.
  if (snapshotOwner !== principal()) project();
  return snapshot;
}
