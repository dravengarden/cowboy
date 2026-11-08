import { useEffect, useMemo, useState } from "react";
import { type CodeDocumentSymbol, fetchCodeOutline } from "../../mobile/review/codeApi";
import { useReviewChecks } from "../../mobile/review/reviewChecksStore";
import { useOwnedReviewDiff } from "../../mobile/review/useOwnedReviewDiff";
import {
  inspectorHunks,
  inspectorOutline,
  inspectorProblems,
  type InspectorHunk,
  type InspectorProblem,
  type InspectorSymbol,
} from "./reviewInspectorModel";
import type { ReviewWorkbench } from "./useReviewWorkbench";

export interface InspectorData {
  readonly hunks: readonly InspectorHunk[];
  readonly outline: readonly InspectorSymbol[] | "unavailable" | undefined;
  readonly problems: readonly InspectorProblem[];
  /** Why diff rows carry no problems or symbols, when they cannot. */
  readonly placement: "source" | "verified" | "unverified" | "none";
  readonly checks: ReturnType<typeof useReviewChecks>;
}

/**
 * The Inspector's data for the open document. A source file's lines are its
 * rows. A working-tree diff places file lines only through the verified
 * projection the phone uses (useOwnedReviewDiff); commits, PRs, committed
 * comparisons and unverified diffs list problems without placing them.
 */
export function useInspectorData(review: ReviewWorkbench): InspectorData {
  const { document, load, target, changes } = review;
  const sessionId = target?.sessionId ?? "";
  const checks = useReviewChecks(sessionId);
  const ready = load?.state === "ready" ? load.value : undefined;
  const workingDiff = document?.kind === "diff" && !document.comparison?.startsWith("committed:");
  const diff = useOwnedReviewDiff(
    sessionId,
    document?.path ?? "",
    !!target && workingDiff && changes.bufferMode === "owned" && !!ready && !ready.truncated,
    document?.kind === "diff" ? document.scope : "unstaged",
    workingDiff ? ready?.text : undefined,
  );
  const placement: InspectorData["placement"] = document?.kind === "source"
    ? "source"
    : workingDiff && diff.status === "matched"
    ? "verified"
    : document?.kind === "diff"
    ? "unverified"
    : "none";
  const rowOf = useMemo((): ((line: number) => number | undefined) => {
    if (placement === "source") return (line) => line;
    if (placement !== "verified" || !ready) return () => undefined;
    const rows = new Map<number, number>();
    ready.text.split("\n").forEach((_, row) => {
      const point = diff.point(row, 1);
      if (point && !rows.has(point.row + 1)) rows.set(point.row + 1, row + 1);
    });
    return (line) => rows.get(line);
  }, [diff, placement, ready]);
  const [symbols, setSymbols] = useState<{ key: string; value: readonly CodeDocumentSymbol[] | "unavailable" }>();
  const outlineKey = document && (document.kind === "source" || workingDiff) ? `${sessionId}\0${document.path}\0${String(changes.dataRevision)}` : undefined;
  useEffect(() => {
    if (!outlineKey || !document) return undefined;
    const controller = new AbortController();
    void fetchCodeOutline(sessionId, document.path, controller.signal).then(
      (outline) => setSymbols({ key: outlineKey, value: outline.symbols }),
      () => {
        if (!controller.signal.aborted) setSymbols({ key: outlineKey, value: "unavailable" });
      },
    );
    return () => controller.abort();
    // The key names the document and worktree revision it reads.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [outlineKey]);
  const observed = symbols?.key === outlineKey ? symbols?.value : undefined;
  return {
    hunks: ready && document?.kind !== "source" ? inspectorHunks(ready.text) : [],
    outline: observed === "unavailable" ? "unavailable" : observed && inspectorOutline(observed, rowOf),
    problems: document ? inspectorProblems(checks, document.path, rowOf) : [],
    placement,
    checks,
  };
}
