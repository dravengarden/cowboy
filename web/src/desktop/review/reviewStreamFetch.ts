import { CodeApiError, fetchCodeDiff, fetchCodeDiffPage } from "../../review/codeApi";
import { invalidateDiffCache, loadCodeDiff } from "../../review/diffCache";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import { entryKey } from "./ReviewChangesList";

export const STREAM_DIFF_PAGES = 4;
export const MAX_STREAM_CACHE = 30;

export interface StreamDiffData {
  readonly state: "loading" | "ready" | "error";
  readonly text?: string | undefined;
  readonly revision?: string | undefined;
  readonly changeFingerprint?: string | undefined;
  readonly added?: number | undefined;
  readonly removed?: number | undefined;
  readonly truncated?: boolean | undefined;
  readonly stale?: boolean | undefined;
  readonly message?: string | undefined;
}

export function streamEntryCacheKey(
  sessionId: string,
  entry: GitReviewEntry,
  contextLines: number,
  showWhitespace: boolean,
  dataRevision: number,
  retryCount = 0,
): string {
  return `${sessionId}\0${entryKey(entry)}\0${entry.change.fingerprint ?? ""}\0${String(contextLines)}\0${showWhitespace ? 1 : 0}\0${String(dataRevision)}\0${String(retryCount)}`;
}

export function isDiffSnapshotTruncated(
  remainingCursor: string | undefined,
  lastPage: { readonly truncated: boolean; readonly nextCursor?: string | undefined; readonly limited?: boolean | undefined },
): boolean {
  return Boolean(remainingCursor) || lastPage.limited === true || (lastPage.truncated && !lastPage.nextCursor);
}

export function isStreamDiffStale(
  entry: GitReviewEntry,
  diff: {
    readonly revision?: string | undefined;
    readonly changeFingerprint?: string | undefined;
    readonly added?: number | undefined;
    readonly removed?: number | undefined;
    readonly truncated?: boolean | undefined;
  },
  priorObserved: { readonly fingerprint: string; readonly revision: string } | undefined,
): boolean {
  const fp = entry.change.fingerprint;
  if (fp && diff.changeFingerprint !== fp) return true;
  if (fp && diff.revision && priorObserved && priorObserved.fingerprint !== fp && priorObserved.revision === diff.revision) return true;
  if (entry.scope === "combined" && !diff.truncated) {
    if (entry.change.additions !== undefined && diff.added !== undefined && diff.added !== entry.change.additions) return true;
    if (entry.change.deletions !== undefined && diff.removed !== undefined && diff.removed !== entry.change.deletions) return true;
  }
  return false;
}

export function selectActiveStreamRows<T extends { readonly key: string }>(
  rows: readonly T[],
  pinnedKeys: readonly string[],
  maxEntries = MAX_STREAM_CACHE,
): readonly T[] {
  if (rows.length <= maxEntries) return rows;
  const byKey = new Map(rows.map((row) => [row.key, row]));
  const chosen = new Set<string>();
  const result: T[] = [];
  for (const key of pinnedKeys) {
    if (result.length >= maxEntries) break;
    const match = byKey.get(key);
    if (match && !chosen.has(key)) {
      chosen.add(key);
      result.push(match);
    }
  }
  for (let i = rows.length - 1; i >= 0 && result.length < maxEntries; i--) {
    const row = rows[i]!;
    if (!chosen.has(row.key)) {
      chosen.add(row.key);
      result.push(row);
    }
  }
  return result;
}

async function fetchSnapshotOnce(
  sessionId: string,
  entry: GitReviewEntry,
  contextLines: number,
  showWhitespace: boolean,
  bypassCache: boolean,
  signal: AbortSignal,
): Promise<StreamDiffData> {
  const fp = entry.change.fingerprint;
  const first = bypassCache
    ? await fetchCodeDiff(sessionId, entry.change.path, contextLines, showWhitespace, entry.scope, entry.comparison, signal, fp)
    : await loadCodeDiff(sessionId, entry.change.path, contextLines, showWhitespace, entry.scope, entry.comparison, signal, fp);
  let text = first.text;
  let cursor = first.nextCursor;
  let lastPage = first;
  for (let page = 1; cursor && page < STREAM_DIFF_PAGES; page++) {
    const next = await fetchCodeDiffPage(sessionId, cursor, signal);
    if (next.revision !== first.revision) throw new CodeApiError(409);
    text += next.text;
    cursor = next.nextCursor;
    lastPage = next;
  }
  return {
    state: "ready",
    text,
    revision: first.revision,
    changeFingerprint: first.changeFingerprint,
    added: first.added,
    removed: first.removed,
    truncated: isDiffSnapshotTruncated(cursor, lastPage),
  };
}

export async function readStreamDiff(
  sessionId: string,
  entry: GitReviewEntry,
  contextLines: number,
  showWhitespace: boolean,
  bypassCache: boolean,
  signal: AbortSignal,
): Promise<StreamDiffData> {
  try {
    return await fetchSnapshotOnce(sessionId, entry, contextLines, showWhitespace, bypassCache, signal);
  } catch (err) {
    if (err instanceof CodeApiError && err.status === 410) {
      invalidateDiffCache(sessionId);
      return await fetchSnapshotOnce(sessionId, entry, contextLines, showWhitespace, true, signal);
    }
    throw err;
  }
}

export function pruneStreamCacheBounded(
  source: ReadonlyMap<string, StreamDiffData>,
  sessionPrefix: string,
  preferKeep: ReadonlySet<string>,
  maxEntries = MAX_STREAM_CACHE,
): Map<string, StreamDiffData> {
  const next = new Map<string, StreamDiffData>();
  for (const [key, value] of source) {
    if (key.startsWith(sessionPrefix)) next.set(key, value);
  }
  while (next.size > maxEntries) {
    let evicted = false;
    for (const key of next.keys()) {
      if (!preferKeep.has(key)) {
        next.delete(key);
        evicted = true;
        break;
      }
    }
    if (!evicted) {
      const oldest = next.keys().next().value;
      if (oldest === undefined) break;
      next.delete(oldest);
    }
  }
  return next;
}
