import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useControlPlaneSessionActivity } from "../../controlPlane";
import { mutateMobileReview } from "../../store";
import {
  type CodeChanges,
  fetchCodeChanges,
  fetchCodeManifest,
} from "../../review/codeApi";
import { invalidateDiffCache } from "../../review/diffCache";
import {
  type GitReviewEntry,
  type GitReviewSection,
  groupGitChanges,
  reviewQueue,
} from "../../review/gitReviewModel";
import { resolveReviewComparison } from "../../review/reviewCompareModel";

export interface DesktopReviewChanges {
  readonly sections: readonly GitReviewSection[];
  /** The complete queue `]f`/`[f` walk, whatever the list filter shows. */
  readonly queue: readonly GitReviewEntry[];
  /** `null` is uncommitted work; `undefined` until the default resolves. */
  readonly comparison: string | null | undefined;
  readonly result: CodeChanges | undefined;
  /** True only when `result` was fetched for the active session and comparison. */
  readonly comparisonMatched: boolean;
  /** Bumped when the worktree changed under an open document. */
  readonly dataRevision: number;
  readonly loading: boolean;
  readonly error: boolean;
  /** How language reads reach this context's buffers (manifest). */
  readonly bufferMode: "legacy" | "owned" | "unavailable" | undefined;
  readonly setComparison: (comparison: string | null) => void;
  readonly refresh: () => void;
}

/** How often the worktree changes are revalidated while Review is shown;
 *  agent activity revalidates sooner. Hidden Review does not poll. */
const MANIFEST_POLL_MS = 10_000;

function changesSignature(changes: CodeChanges): string {
  const parts = changes.changes.map((c) =>
    `${c.path}\0${c.oldPath ?? ""}\0${c.status}\0${c.staged ? 1 : 0}\0${c.unstaged ? 1 : 0}\0${c.fingerprint ?? ""}\0${c.additions ?? ""}\0${c.deletions ?? ""}`
  );
  return `${changes.revision}\0${changes.head ?? ""}\0${changes.comparison ?? ""}\0${parts.join("\n")}`;
}

/**
 * The Git change list of `sessionId` under the synced comparison: `""` is
 * uncommitted work, absent follows the repository default (13-code-review
 * "Change target"). Desktop reads and writes only the comparison and never
 * the phone's tabs.
 */
function useReviewChangesRevalidation(input: {
  readonly active: boolean;
  readonly activity: unknown;
  readonly current: string | null | undefined;
  readonly sessionId: string | undefined;
  readonly manifestRevision: { current: string | undefined };
  readonly lastSignature: { current: string | undefined };
  readonly setBufferMode: (value: { sessionId: string; mode: DesktopReviewChanges["bufferMode"] }) => void;
  readonly setResult: (value: { sessionId: string; comparison: string | null; value: CodeChanges }) => void;
  readonly setDataRevision: React.Dispatch<React.SetStateAction<number>>;
}): void {
  const { active, activity, current, sessionId, manifestRevision, lastSignature, setBufferMode, setResult, setDataRevision } = input;
  useEffect(() => {
    if (!active || !sessionId || current === undefined) return undefined;
    let controller: AbortController | undefined;
    const check = (): void => {
      controller?.abort();
      controller = new AbortController();
      const observer = controller;
      void (async () => {
        try {
          const [manifest, fresh] = await Promise.all([
            fetchCodeManifest(sessionId, observer.signal),
            fetchCodeChanges(sessionId, observer.signal, current ?? undefined),
          ]);
          if (observer.signal.aborted) return;
          setBufferMode({ sessionId, mode: manifest.bufferMode });
          const prevManifest = manifestRevision.current;
          manifestRevision.current = manifest.revision;
          const nextSig = changesSignature(fresh);
          const prevSig = lastSignature.current;
          if ((prevManifest && prevManifest !== manifest.revision) || (prevSig && prevSig !== nextSig)) {
            lastSignature.current = nextSig;
            invalidateDiffCache(sessionId);
            setResult({ sessionId, comparison: current, value: fresh });
            setDataRevision((value) => value + 1);
          } else if (!prevSig) {
            lastSignature.current = nextSig;
          }
        } catch {
          // Best-effort background revalidation; ignore transient errors.
        }
      })();
    };
    const debounce = globalThis.setTimeout(check, 600);
    const timer = globalThis.setInterval(check, MANIFEST_POLL_MS);
    return () => {
      globalThis.clearTimeout(debounce);
      globalThis.clearInterval(timer);
      controller?.abort();
    };
  }, [active, activity, current, lastSignature, manifestRevision, sessionId, setBufferMode, setDataRevision, setResult]);
}

export function useDesktopReviewChanges(
  sessionId: string | undefined,
  stateKey: string | undefined,
  preferred: string | undefined,
  active: boolean,
): DesktopReviewChanges {
  const preferredComparison = preferred === undefined ? undefined : preferred || null;
  const [comparison, setLocalComparison] = useState<{
    sessionId: string | undefined;
    value: string | null | undefined;
  }>({ sessionId, value: undefined });
  const current = comparison.sessionId === sessionId ? comparison.value : undefined;
  const [result, setResult] = useState<{
    sessionId: string;
    comparison: string | null;
    value: CodeChanges;
  }>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [dataRevision, setDataRevision] = useState(0);
  const [refreshRequest, setRefreshRequest] = useState(0);
  const manifestRevision = useRef<string | undefined>(undefined);
  const lastSignature = useRef<string | undefined>(undefined);
  const [bufferMode, setBufferMode] = useState<{ sessionId: string; mode: DesktopReviewChanges["bufferMode"] }>();
  const activity = useControlPlaneSessionActivity(sessionId);

  useEffect(() => {
    manifestRevision.current = undefined;
    lastSignature.current = undefined;
  }, [sessionId, current]);

  // Another device or an agent changed the synced comparison.
  useEffect(() => {
    if (preferredComparison === undefined) return;
    setLocalComparison((previous) =>
      previous.sessionId === sessionId && previous.value === preferredComparison
        ? previous
        : { sessionId, value: preferredComparison }
    );
  }, [preferredComparison, sessionId]);

  useEffect(() => {
    if (!active || !sessionId) return undefined;
    const controller = new AbortController();
    const { signal } = controller;
    setLoading(true);
    void (async () => {
      try {
        let resolvedComparison = current;
        let changes = await fetchCodeChanges(sessionId, signal, current ?? undefined);
        if (current === undefined) {
          resolvedComparison = resolveReviewComparison(
            preferredComparison,
            changes.comparisons,
            changes.defaultComparison,
          );
          if (resolvedComparison !== null) {
            changes = await fetchCodeChanges(sessionId, signal, resolvedComparison);
          }
          if (signal.aborted) return;
          setLocalComparison({ sessionId, value: resolvedComparison });
        }
        if (signal.aborted) return;
        lastSignature.current = changesSignature(changes);
        setResult({ sessionId, comparison: resolvedComparison ?? null, value: changes });
        setError(false);
      } catch (reason) {
        if (!signal.aborted && !(reason instanceof DOMException && reason.name === "AbortError")) {
          setError(true);
        }
      } finally {
        if (!signal.aborted) setLoading(false);
      }
    })();
    return () => controller.abort();
    // `preferredComparison` only seeds the first resolution; a later change
    // arrives through `current`.
  }, [active, current, dataRevision, refreshRequest, sessionId]);

  useReviewChangesRevalidation({
    active,
    activity,
    current,
    sessionId,
    manifestRevision,
    lastSignature,
    setBufferMode,
    setResult,
    setDataRevision,
  });

  const setComparison = useCallback((next: string | null): void => {
    if (!sessionId) return;
    invalidateDiffCache(sessionId);
    setLocalComparison({ sessionId, value: next });
    if (stateKey) mutateMobileReview(stateKey, "setComparison", { comparison: next ?? "" });
  }, [sessionId, stateKey]);
  const refresh = useCallback((): void => {
    if (sessionId) invalidateDiffCache(sessionId);
    setRefreshRequest((value) => value + 1);
  }, [sessionId]);

  const comparisonMatched =
    result !== undefined &&
    result.sessionId === sessionId &&
    current !== undefined &&
    result.comparison === current;
  const value = comparisonMatched ? result.value : undefined;
  const sections = useMemo(
    () => value ? groupGitChanges(value.changes, current ?? undefined) : [],
    [current, value],
  );
  const queue = useMemo(() => reviewQueue(sections), [sections]);
  return {
    sections,
    queue,
    comparison: current,
    result: value,
    comparisonMatched,
    dataRevision,
    loading,
    error,
    bufferMode: bufferMode?.sessionId === sessionId ? bufferMode?.mode : undefined,
    setComparison,
    refresh,
  };
}
