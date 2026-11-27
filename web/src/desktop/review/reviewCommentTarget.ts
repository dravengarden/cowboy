import { diffSelectionLines, type ReviewLineSelection } from "../../review/reviewLineSelection";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import { reviewDocumentKey } from "./desktopReviewTabs";
import { pullFile } from "./pullPatches";
import type { ReviewCommentSide } from "./reviewCommentsStore";
import { editorSelection } from "./reviewEditorSelection";
import type { ReviewCommentTarget } from "./ReviewInlineCommentBar";
import type { ReviewWorkbench } from "./useReviewWorkbench";

export interface ResolvedDiffLocation {
  readonly startLine: number;
  readonly endLine: number;
  readonly side: ReviewCommentSide;
}

/** Map a contiguous unified-diff row selection to real file lines and side. */
export function resolveDiffCommentLocation(
  unifiedText: string,
  selection: ReviewLineSelection,
): ResolvedDiffLocation {
  const from = Math.max(1, Math.min(selection.from, selection.to));
  const to = Math.max(from, Math.max(selection.from, selection.to));
  const covered = diffSelectionLines(unifiedText, { from, to });
  if (covered.new.length > 0) {
    return { startLine: covered.new[0]!, endLine: covered.new.at(-1)!, side: "new" };
  }
  if (covered.old.length > 0) {
    return { startLine: covered.old[0]!, endLine: covered.old.at(-1)!, side: "old" };
  }
  return { startLine: from, endLine: to, side: "patch" };
}

/** Map an arbitrary set of selected unified patch rows (such as non-monotonic
 *  rows mapped from a side-by-side split diff) to exact file lines and snippet
 *  without absorbing unselected intermediate lines. */
export function resolveSelectedUnifiedRows(
  unifiedText: string,
  selectedRows: readonly number[],
): ResolvedDiffLocation & { readonly snippet: string } {
  const uniqueSorted = [...new Set(selectedRows.filter((r) => Number.isInteger(r) && r > 0))].sort((a, b) => a - b);
  const lines = unifiedText.split("\n");
  if (uniqueSorted.length === 0) {
    return { startLine: 1, endLine: 1, side: "patch", snippet: "" };
  }
  const newLines: number[] = [];
  const oldLines: number[] = [];
  for (const row of uniqueSorted) {
    const covered = diffSelectionLines(unifiedText, { from: row, to: row });
    for (const n of covered.new) newLines.push(n);
    for (const o of covered.old) oldLines.push(o);
  }
  const snippet = uniqueSorted
    .map((row) => lines[row - 1])
    .filter((line): line is string => line !== undefined)
    .join("\n");
  if (newLines.length > 0) {
    return { startLine: Math.min(...newLines), endLine: Math.max(...newLines), side: "new", snippet };
  }
  if (oldLines.length > 0) {
    return { startLine: Math.min(...oldLines), endLine: Math.max(...oldLines), side: "old", snippet };
  }
  return { startLine: uniqueSorted[0]!, endLine: uniqueSorted.at(-1)!, side: "patch", snippet };
}

/** Canonical provenance identity used for draft lookup, composer reset, and
 *  context-change dismissal across diffs, commits, PRs, and comparisons. */
export function commentTargetIdentity(
  target: Pick<
    ReviewCommentTarget,
    | "ownerSessionId"
    | "reviewSessionId"
    | "kind"
    | "path"
    | "scope"
    | "comparison"
    | "commitOid"
    | "pullLabel"
    | "revision"
    | "startLine"
    | "endLine"
    | "side"
  >,
): string {
  return [
    target.ownerSessionId,
    target.reviewSessionId,
    target.kind,
    target.path,
    target.scope ?? "",
    target.comparison ?? "",
    target.commitOid ?? "",
    target.pullLabel ?? "",
    target.revision ?? "",
    `${String(target.startLine)}:${String(target.endLine)}:${target.side ?? "new"}`,
  ].join("\0");
}

/** True only when the open document, loaded snapshot, and mounted editor all
 *  represent the exact same document key. */
export function isDocumentBufferReady(review: ReviewWorkbench): boolean {
  const doc = review.document;
  if (!doc || !review.target || !review.context.agentSessionId) return false;
  if (review.load?.state !== "ready") return false;
  const expectedKey = reviewDocumentKey(doc);
  if (review.load.value.key !== expectedKey) return false;
  const editorDom = review.editor.current?.dom;
  const host = editorDom?.closest<HTMLElement>("[data-review-document-key]");
  if (host && host.dataset.reviewDocumentKey !== expectedKey) return false;
  return true;
}

export function buildEditorCommentTarget(review: ReviewWorkbench): ReviewCommentTarget | null {
  if (!isDocumentBufferReady(review)) return null;
  const doc = review.document!;
  const ownerSessionId = review.context.agentSessionId!;
  const reviewSessionId = review.target!.sessionId;
  const ready = review.load!.state === "ready" ? review.load!.value : undefined;
  if (!ready) return null;
  const view = review.editor.current;
  const sel = view ? editorSelection(view) : { from: review.cursorLine, to: review.cursorLine };

  if (doc.kind === "source") {
    const lines = (view ? view.state.doc.toString() : ready.text).split("\n");
    const from = Math.max(1, Math.min(sel.from, sel.to));
    const to = Math.max(from, Math.max(sel.from, sel.to));
    return {
      ownerSessionId,
      reviewSessionId,
      path: doc.path,
      startLine: from,
      endLine: to,
      side: "new",
      codeSnippet: lines.slice(from - 1, Math.min(lines.length, to)).join("\n"),
      kind: "source",
      root: review.root,
      revision: ready.revision,
    };
  }

  const unifiedText = ready.text;
  const unifiedRows = review.split?.unifiedRows;
  let startLine: number;
  let endLine: number;
  let side: ReviewCommentSide;
  let snippet: string;
  if (unifiedRows) {
    const fromSplit = Math.max(1, Math.min(sel.from, sel.to));
    const toSplit = Math.max(fromSplit, Math.max(sel.from, sel.to));
    const picked: number[] = [];
    for (let row = fromSplit; row <= toSplit; row++) {
      const mapped = unifiedRows[row - 1];
      if (mapped !== undefined && mapped > 0) picked.push(mapped);
    }
    const resolved = resolveSelectedUnifiedRows(unifiedText, picked);
    startLine = resolved.startLine;
    endLine = resolved.endLine;
    side = resolved.side;
    snippet = resolved.snippet;
  } else {
    const loc = resolveDiffCommentLocation(unifiedText, sel);
    startLine = loc.startLine;
    endLine = loc.endLine;
    side = loc.side;
    const lines = unifiedText.split("\n");
    const sliceFrom = Math.max(1, Math.min(sel.from, sel.to));
    const sliceTo = Math.max(sliceFrom, Math.max(sel.from, sel.to));
    snippet = lines.slice(sliceFrom - 1, Math.min(lines.length, sliceTo)).join("\n");
  }

  const pullRef = doc.kind === "pr"
    ? pullFile(doc.repositoryId, doc.number, doc.revision, doc.path)
    : undefined;
  const pullLabel = doc.kind === "pr"
    ? `${pullRef ? `${pullRef.binding.owner}/${pullRef.binding.repository}` : doc.repositoryId}#${String(doc.number)}`
    : undefined;

  return {
    ownerSessionId,
    reviewSessionId,
    path: doc.path,
    startLine,
    endLine,
    side,
    codeSnippet: snippet,
    kind: doc.kind,
    root: review.root,
    comparison: doc.kind === "diff" ? doc.comparison : undefined,
    scope: doc.kind === "diff" ? doc.scope : undefined,
    commitOid: doc.kind === "commit" ? doc.oid : undefined,
    pullLabel,
    revision: ready.revision,
  };
}

export function buildStreamCommentTarget(input: {
  readonly ownerSessionId: string;
  readonly reviewSessionId: string;
  readonly entry: GitReviewEntry;
  readonly diffText: string;
  readonly revision: string | undefined;
  readonly lineIndex: number;
  readonly root: string | undefined;
}): ReviewCommentTarget {
  const row = input.lineIndex + 1;
  const loc = resolveDiffCommentLocation(input.diffText, { from: row, to: row });
  const lines = input.diffText.split("\n");
  const snippet = lines.slice(Math.max(0, input.lineIndex - 1), Math.min(lines.length, input.lineIndex + 2)).join("\n");
  return {
    ownerSessionId: input.ownerSessionId,
    reviewSessionId: input.reviewSessionId,
    path: input.entry.change.path,
    startLine: loc.startLine,
    endLine: loc.endLine,
    side: loc.side,
    codeSnippet: snippet,
    kind: "diff",
    root: input.root,
    comparison: input.entry.comparison,
    scope: input.entry.scope,
    revision: input.revision,
  };
}
