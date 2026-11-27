import { diffSelectionLines, type ReviewLineSelection } from "../../review/reviewLineSelection";
import type { GitReviewEntry } from "../../review/gitReviewModel";
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

/** Map a unified-diff row selection to real file lines and side, matching
 *  Cowboy's `reviewSelectionMessage` semantics. */
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

export function commentDraftKey(
  target: Pick<ReviewCommentTarget, "ownerSessionId" | "reviewSessionId" | "path" | "startLine" | "endLine" | "side">,
): string {
  return `${target.ownerSessionId}\0${target.reviewSessionId}\0${target.path}\0${String(target.startLine)}:${String(target.endLine)}:${target.side ?? "new"}`;
}

export function buildEditorCommentTarget(review: ReviewWorkbench): ReviewCommentTarget | null {
  const doc = review.document;
  const ownerSessionId = review.context.agentSessionId;
  const reviewSessionId = review.target?.sessionId;
  if (!doc || !ownerSessionId || !reviewSessionId) return null;
  const view = review.editor.current;
  const ready = review.load?.state === "ready" ? review.load.value : undefined;
  const sel = view ? editorSelection(view) : { from: review.cursorLine, to: review.cursorLine };

  if (doc.kind === "source") {
    const lines = (view ? view.state.doc.toString() : (ready?.text ?? "")).split("\n");
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
      revision: ready?.revision,
    };
  }

  const unifiedText = ready?.text ?? (view ? view.state.doc.toString() : "");
  const unifiedRows = review.split?.unifiedRows;
  const unifiedSel: ReviewLineSelection = unifiedRows && ready
    ? {
      from: Math.min(unifiedRows[sel.from - 1] ?? 1, unifiedRows[sel.to - 1] ?? 1),
      to: Math.max(unifiedRows[sel.from - 1] ?? 1, unifiedRows[sel.to - 1] ?? 1),
    }
    : sel;
  const loc = resolveDiffCommentLocation(unifiedText, unifiedSel);
  const lines = unifiedText.split("\n");
  const sliceFrom = Math.max(1, Math.min(unifiedSel.from, unifiedSel.to));
  const sliceTo = Math.max(sliceFrom, Math.max(unifiedSel.from, unifiedSel.to));
  const snippet = lines.slice(sliceFrom - 1, Math.min(lines.length, sliceTo)).join("\n");
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
    startLine: loc.startLine,
    endLine: loc.endLine,
    side: loc.side,
    codeSnippet: snippet,
    kind: doc.kind,
    root: review.root,
    comparison: doc.kind === "diff" ? doc.comparison : undefined,
    scope: doc.kind === "diff" ? doc.scope : undefined,
    commitOid: doc.kind === "commit" ? doc.oid : undefined,
    pullLabel,
    revision: ready?.revision,
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
