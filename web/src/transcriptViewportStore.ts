export type TranscriptViewportMode = "history" | "page";

// Device-local, runtime-only reading positions. They deliberately do not own
// session state and do not survive a PWA reload/relaunch. A bounded TTL/LRU map
// is enough to make ordinary session round-trips feel continuous without
// retaining transcript data or accumulating one entry per historical session.
export interface TranscriptViewport {
  sessionId: string;
  mode: TranscriptViewportMode;
  pageId: string | null;
  anchorKey: string | null;
  anchorOffset: number;
  scrollOffset: number;
  following: boolean;
  touchedAt: number;
}

const TTL_MS = 24 * 60 * 60 * 1_000;
const MAX_ENTRIES = 32;
const STORAGE_KEY = "cowboy:transcript-viewports:v1";
const entries = new Map<string, TranscriptViewport>();

function key(sessionId: string, mode: TranscriptViewportMode): string {
  return `${sessionId}:${mode}`;
}

function persist(): void {
  if (typeof sessionStorage === "undefined") return;
  try {
    sessionStorage.setItem(
      STORAGE_KEY,
      JSON.stringify([...entries.values()]),
    );
  } catch {
    // Reading continuity is best-effort and must never block the transcript.
  }
}

function restore(): void {
  if (typeof sessionStorage === "undefined") return;
  try {
    const stored = JSON.parse(sessionStorage.getItem(STORAGE_KEY) ?? "[]") as TranscriptViewport[];
    for (const entry of stored) {
      if (
        entry &&
        typeof entry.sessionId === "string" &&
        (entry.mode === "history" || entry.mode === "page") &&
        typeof entry.scrollOffset === "number" &&
        typeof entry.touchedAt === "number"
      ) {
        entries.set(key(entry.sessionId, entry.mode), entry);
      }
    }
  } catch {
    sessionStorage.removeItem(STORAGE_KEY);
  }
}

function prune(now = Date.now()): void {
  let changed = false;
  for (const [entryKey, entry] of entries) {
    if (now - entry.touchedAt > TTL_MS) {
      entries.delete(entryKey);
      changed = true;
    }
  }
  const overflow = entries.size - MAX_ENTRIES;
  if (overflow <= 0) {
    if (changed) persist();
    return;
  }
  const oldest = [...entries.entries()].sort(
    (a, b) => a[1].touchedAt - b[1].touchedAt,
  );
  for (let index = 0; index < overflow; index++) {
    const entryKey = oldest[index]?.[0];
    if (entryKey) {
      entries.delete(entryKey);
      changed = true;
    }
  }
  if (changed) persist();
}

restore();

export function saveTranscriptViewport(
  entry: Omit<TranscriptViewport, "touchedAt">,
  now = Date.now(),
): void {
  entries.set(key(entry.sessionId, entry.mode), { ...entry, touchedAt: now });
  prune(now);
  persist();
}

export function getTranscriptViewport(
  sessionId: string,
  mode: TranscriptViewportMode,
  now = Date.now(),
): TranscriptViewport | null {
  prune(now);
  const entry = entries.get(key(sessionId, mode));
  if (!entry) return null;
  entry.touchedAt = now;
  return { ...entry };
}

export function canRestoreTranscriptViewport(
  entry: TranscriptViewport | null,
  mode: TranscriptViewportMode,
  pageId: string | null,
): entry is TranscriptViewport {
  return entry !== null &&
    (mode === "history" || entry.pageId === pageId);
}

export function clearTranscriptViewport(
  sessionId: string,
  mode?: TranscriptViewportMode,
): void {
  if (mode) entries.delete(key(sessionId, mode));
  else {
    entries.delete(key(sessionId, "history"));
    entries.delete(key(sessionId, "page"));
  }
  persist();
}

export function retainTranscriptViewportSessions(
  sessionIds: ReadonlySet<string>,
): void {
  for (const [entryKey, entry] of entries) {
    if (!sessionIds.has(entry.sessionId)) entries.delete(entryKey);
  }
  persist();
}

export function resetTranscriptViewportStoreForTest(): void {
  entries.clear();
  persist();
}
