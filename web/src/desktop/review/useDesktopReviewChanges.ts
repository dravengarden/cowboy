import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useControlPlaneSessionActivity } from "../../controlPlane";
import { mutateMobileReview } from "../../store";
import {
  type CodeChanges,
  fetchCodeChanges,
  fetchCodeManifest,
} from "../../mobile/review/codeApi";
import { invalidateDiffCache } from "../../mobile/review/diffCache";
import {
  type GitReviewEntry,
  type GitReviewSection,
  groupGitChanges,
  reviewQueue,
} from "../../mobile/review/gitReviewModel";
import { resolveReviewComparison } from "../../mobile/review/reviewCompareModel";

export interface DesktopReviewChanges {
  readonly sections: readonly GitReviewSection[];
  /** The complete queue `]f`/`[f` walk, whatever the list filter shows. */
  readonly queue: readonly GitReviewEntry[];
  /** `null` is uncommitted work; `undefined` until the default resolves. */
  readonly comparison: string | null | undefined;
  readonly result: CodeChanges | undefined;
  /** Bumped when the worktree changed under an open document. */
  readonly dataRevision: number;
  readonly loading: boolean;
  readonly error: boolean;
  /** How language reads reach this context's buffers (manifest). */
  readonly bufferMode: "legacy" | "owned" | "unavailable" | undefined;
  readonly setComparison: (comparison: string | null) => void;
  readonly refresh: () => void;
}

/** How often the manifest is revalidated while Review is shown; agent
 *  activity revalidates sooner. Hidden Review does not poll. */
const MANIFEST_POLL_MS = 10_000;

/**
 * The Git change list of `sessionId` under the synced comparison: `""` is
 * uncommitted work, absent follows the repository default (13-code-review
 * "Change target"). Desktop reads and writes only the comparison and never
 * the phone's tabs.
 */
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
  const [result, setResult] = useState<{ sessionId: string; value: CodeChanges }>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [dataRevision, setDataRevision] = useState(0);
  const [refreshRequest, setRefreshRequest] = useState(0);
  const manifestRevision = useRef<string | undefined>(undefined);
  const [bufferMode, setBufferMode] = useState<{ sessionId: string; mode: DesktopReviewChanges["bufferMode"] }>();
  const activity = useControlPlaneSessionActivity(sessionId);

  useEffect(() => {
    manifestRevision.current = undefined;
  }, [sessionId]);

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
        let changes = await fetchCodeChanges(sessionId, signal, current ?? undefined);
        if (current === undefined) {
          const resolved = resolveReviewComparison(
            preferredComparison,
            changes.comparisons,
            changes.defaultComparison,
          );
          if (resolved !== null) {
            changes = await fetchCodeChanges(sessionId, signal, resolved);
          }
          if (signal.aborted) return;
          setLocalComparison({ sessionId, value: resolved });
        }
        if (signal.aborted) return;
        setResult({ sessionId, value: changes });
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

  // Revalidate the worktree: on agent activity (debounced) and on a timer.
  useEffect(() => {
    if (!active || !sessionId) return undefined;
    let controller: AbortController | undefined;
    const check = (): void => {
      controller?.abort();
      controller = new AbortController();
      const observer = controller;
      void fetchCodeManifest(sessionId, observer.signal).then((manifest) => {
        if (observer.signal.aborted) return;
        setBufferMode({ sessionId, mode: manifest.bufferMode });
        const previous = manifestRevision.current;
        manifestRevision.current = manifest.revision;
        if (previous && previous !== manifest.revision) {
          invalidateDiffCache(sessionId);
          setDataRevision((value) => value + 1);
        }
      }).catch(() => undefined);
    };
    const debounce = globalThis.setTimeout(check, 600);
    const timer = globalThis.setInterval(check, MANIFEST_POLL_MS);
    return () => {
      globalThis.clearTimeout(debounce);
      globalThis.clearInterval(timer);
      controller?.abort();
    };
  }, [active, activity, sessionId]);

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

  const value = result?.sessionId === sessionId ? result?.value : undefined;
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
    dataRevision,
    loading,
    error,
    bufferMode: bufferMode?.sessionId === sessionId ? bufferMode?.mode : undefined,
    setComparison,
    refresh,
  };
}
