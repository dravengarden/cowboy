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
  /** Owner-plus-target identity scoping comparison and UI filters. */
  readonly ownerKey: string;
  readonly result: CodeChanges | undefined;
  /** True only when `result` matches the active owner, target session, and comparison. */
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

function useReviewChangesRevalidation(input: {
  readonly active: boolean;
  readonly activity: unknown;
  readonly ownerKey: string;
  readonly current: string | null | undefined;
  readonly preferredComparison: string | null | undefined;
  readonly hasResult: boolean;
  readonly sessionId: string | undefined;
  readonly manifestRevision: { current: string | undefined };
  readonly lastSignature: { current: string | undefined };
  readonly setLocalComparison: (value: { ownerKey: string; value: string | null | undefined }) => void;
  readonly setBufferMode: (value: { sessionId: string; mode: DesktopReviewChanges["bufferMode"] }) => void;
  readonly setResult: (value: { ownerKey: string; sessionId: string; comparison: string | null; value: CodeChanges }) => void;
  readonly setError: (value: boolean) => void;
  readonly setDataRevision: React.Dispatch<React.SetStateAction<number>>;
}): void {
  const { active, activity, ownerKey, current, preferredComparison, hasResult, sessionId, manifestRevision, lastSignature, setLocalComparison, setBufferMode, setResult, setError, setDataRevision } = input;
  useEffect(() => {
    if (!active || !sessionId) return undefined;
    let controller: AbortController | undefined;
    const check = (): void => {
      controller?.abort();
      controller = new AbortController();
      const observer = controller;
      void (async () => {
        const [manifestRes, changesRes] = await Promise.allSettled([
          fetchCodeManifest(sessionId, observer.signal),
          (async () => {
            let resolvedComparison = current;
            let fresh = await fetchCodeChanges(sessionId, observer.signal, current ?? undefined);
            if (current === undefined) {
              resolvedComparison = resolveReviewComparison(preferredComparison, fresh.comparisons, fresh.defaultComparison);
              if (resolvedComparison !== null) fresh = await fetchCodeChanges(sessionId, observer.signal, resolvedComparison);
            }
            return { resolvedComparison: resolvedComparison ?? null, fresh };
          })(),
        ]);
        if (observer.signal.aborted) return;
        let invalidated = false;
        if (manifestRes.status === "fulfilled") {
          setBufferMode({ sessionId, mode: manifestRes.value.bufferMode });
          const prevManifest = manifestRevision.current;
          manifestRevision.current = manifestRes.value.revision;
          if (prevManifest && prevManifest !== manifestRes.value.revision) invalidated = true;
        }
        if (changesRes.status === "fulfilled") {
          const { resolvedComparison, fresh } = changesRes.value;
          const nextSig = changesSignature(fresh);
          const prevSig = lastSignature.current;
          const changed = Boolean(prevSig && prevSig !== nextSig);
          lastSignature.current = nextSig;
          if (current === undefined) setLocalComparison({ ownerKey, value: resolvedComparison });
          if (!hasResult || changed || invalidated) {
            setResult({ ownerKey, sessionId, comparison: resolvedComparison, value: fresh });
            setError(false);
          }
          if (changed || invalidated) {
            invalidateDiffCache(sessionId);
            setDataRevision((value) => value + 1);
          }
        } else if (invalidated) {
          invalidateDiffCache(sessionId);
          setDataRevision((value) => value + 1);
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
  }, [active, activity, current, hasResult, lastSignature, manifestRevision, ownerKey, preferredComparison, sessionId, setBufferMode, setDataRevision, setError, setLocalComparison, setResult]);
}

/**
 * The Git change list of `sessionId` under the synced comparison: `""` is
 * uncommitted work, absent follows the repository default (13-code-review
 * "Change target"). Scoped by owner+target (`stateKey`) so switching between
 * Agent sessions browsing the same worktree never leaks comparisons or filters.
 */
export function useDesktopReviewChanges(
  sessionId: string | undefined,
  stateKey: string | undefined,
  preferred: string | undefined,
  active: boolean,
): DesktopReviewChanges {
  const ownerKey = stateKey ?? sessionId ?? "";
  const preferredComparison = preferred === undefined ? undefined : preferred || null;
  const [comparison, setLocalComparison] = useState<{
    ownerKey: string;
    value: string | null | undefined;
  }>({ ownerKey, value: undefined });
  const current = comparison.ownerKey === ownerKey ? comparison.value : undefined;
  const [result, setResult] = useState<{
    ownerKey: string;
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
  }, [ownerKey, sessionId, current]);

  // Another device or an agent changed the synced comparison.
  useEffect(() => {
    if (preferredComparison === undefined) return;
    setLocalComparison((previous) =>
      previous.ownerKey === ownerKey && previous.value === preferredComparison
        ? previous
        : { ownerKey, value: preferredComparison }
    );
  }, [ownerKey, preferredComparison]);

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
          setLocalComparison({ ownerKey, value: resolvedComparison });
        }
        if (signal.aborted) return;
        const nextSig = changesSignature(changes);
        const prevSig = lastSignature.current;
        lastSignature.current = nextSig;
        setResult({ ownerKey, sessionId, comparison: resolvedComparison ?? null, value: changes });
        if (prevSig && prevSig !== nextSig) {
          invalidateDiffCache(sessionId);
          setDataRevision((value) => value + 1);
        }
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
  }, [active, current, dataRevision, ownerKey, refreshRequest, sessionId]);

  const comparisonMatched =
    result !== undefined &&
    result.ownerKey === ownerKey &&
    result.sessionId === sessionId &&
    current !== undefined &&
    result.comparison === current;

  useReviewChangesRevalidation({
    active, activity, ownerKey, current, preferredComparison,
    hasResult: comparisonMatched, sessionId, manifestRevision, lastSignature,
    setLocalComparison, setBufferMode, setResult, setError, setDataRevision,
  });

  const setComparison = useCallback((next: string | null): void => {
    if (!sessionId) return;
    invalidateDiffCache(sessionId);
    setLocalComparison({ ownerKey, value: next });
    if (stateKey) mutateMobileReview(stateKey, "setComparison", { comparison: next ?? "" });
  }, [ownerKey, sessionId, stateKey]);
  const refresh = useCallback((): void => {
    if (sessionId) invalidateDiffCache(sessionId);
    setRefreshRequest((value) => value + 1);
  }, [sessionId]);


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
    ownerKey,
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
