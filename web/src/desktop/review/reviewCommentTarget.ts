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

const HUNK_HEADER = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

export function resolveDiffCommentLocation(
  unifiedText: string,
  selection: ReviewLineSelection,
): ResolvedDiffLocation {
  const from = Math.max(1, Math.min(selection.from, selection.to));
  const to = Math.max(from, Math.max(selection.from, selection.to));
  const covered = diffSelectionLines(unifiedText, { from, to });
  if (covered.new.length > 0) return { startLine: covered.new[0]!, endLine: covered.new.at(-1)!, side: "new" };
  if (covered.old.length > 0) return { startLine: covered.old[0]!, endLine: covered.old.at(-1)!, side: "old" };
  return { startLine: from, endLine: to, side: "patch" };
}

/** Single linear pass resolving arbitrary selected unified patch rows. */
export function resolveSelectedUnifiedRows(
  unifiedText: string,
  selectedRows: readonly number[],
): ResolvedDiffLocation & { readonly snippet: string } {
  const selectedSet = new Set(selectedRows.filter((r) => Number.isInteger(r) && r > 0));
  if (selectedSet.size === 0) return { startLine: 1, endLine: 1, side: "patch", snippet: "" };
  const lines = unifiedText.split("\n");
  let oldLine: number | undefined;
  let newLine: number | undefined;
  let minNew = Infinity;
  let maxNew = -Infinity;
  let minOld = Infinity;
  let maxOld = -Infinity;
  let minPatch = Infinity;
  let maxPatch = -Infinity;
  const pickedLines: string[] = [];

  for (let index = 0; index < lines.length; index++) {
    const row = index + 1;
    const line = lines[index]!;
    const picked = selectedSet.has(row);
    if (picked) {
      pickedLines.push(line);
      if (row < minPatch) minPatch = row;
      if (row > maxPatch) maxPatch = row;
    }
    const hunk = HUNK_HEADER.exec(line);
    if (hunk) {
      oldLine = Number(hunk[1]);
      newLine = Number(hunk[2]);
      continue;
    }
    if (oldLine === undefined || newLine === undefined || line.startsWith("\\")) continue;
    if (line.startsWith("+")) {
      if (picked) {
        if (newLine < minNew) minNew = newLine;
        if (newLine > maxNew) maxNew = newLine;
      }
      newLine += 1;
    } else if (line.startsWith("-")) {
      if (picked) {
        if (oldLine < minOld) minOld = oldLine;
        if (oldLine > maxOld) maxOld = oldLine;
      }
      oldLine += 1;
    } else {
      if (picked) {
        if (newLine < minNew) minNew = newLine;
        if (newLine > maxNew) maxNew = newLine;
      }
      oldLine += 1;
      newLine += 1;
    }
  }

  const snippet = pickedLines.join("\n");
  if (Number.isFinite(minNew)) return { startLine: minNew, endLine: maxNew, side: "new", snippet };
  if (Number.isFinite(minOld)) return { startLine: minOld, endLine: maxOld, side: "old", snippet };
  return {
    startLine: Number.isFinite(minPatch) ? minPatch : 1,
    endLine: Number.isFinite(maxPatch) ? maxPatch : 1,
    side: "patch",
    snippet,
  };
}

export function commentTargetIdentity(
  target: Pick<
    ReviewCommentTarget,
    | "ownerSessionId" | "reviewSessionId" | "kind" | "path" | "scope" | "comparison"
    | "commitOid" | "pullLabel" | "revision" | "startLine" | "endLine" | "side"
  >,
): string {
  return [
    target.ownerSessionId, target.reviewSessionId, target.kind, target.path,
    target.scope ?? "", target.comparison ?? "", target.commitOid ?? "",
    target.pullLabel ?? "", target.revision ?? "",
    `${String(target.startLine)}:${String(target.endLine)}:${target.side ?? "new"}`,
  ].join("\0");
}

/** True only when the open document, loaded snapshot, and mounted editor
 *  (unified or `:split`) all represent the exact same document key. */
export function isDocumentBufferReady(review: ReviewWorkbench): boolean {
  const doc = review.document;
  if (!doc || !review.target || !review.context.agentSessionId) return false;
  if (review.load?.state !== "ready") return false;
  const expectedKey = reviewDocumentKey(doc);
  if (review.load.value.key !== expectedKey) return false;
  const expectedMountedKey = review.split ? `${expectedKey}:split` : expectedKey;
  const editorDom = review.editor.current?.dom;
  const host = editorDom?.closest<HTMLElement>("[data-review-document-key]");
  if (host && host.dataset.reviewDocumentKey !== expectedMountedKey) return false;
  return true;
}

function externalContextMeta(review: ReviewWorkbench) {
  const target = review.target;
  if (!target || target.own) return {};
  return {
    root: review.root,
    machineId: target.machineId,
    contextLabel: target.label,
    reviewContextId: target.sessionId,
  };
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
  const ext = externalContextMeta(review);

  if (doc.kind === "source") {
    const lines = (view ? view.state.doc.toString() : ready.text).split("\n");
    const from = Math.max(1, Math.min(sel.from, sel.to));
    const to = Math.max(from, Math.max(sel.from, sel.to));
    return {
      ownerSessionId, reviewSessionId, path: doc.path, startLine: from, endLine: to, side: "new",
      codeSnippet: lines.slice(from - 1, Math.min(lines.length, to)).join("\n"),
      kind: "source", ...ext, revision: ready.revision,
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
    ownerSessionId, reviewSessionId, path: doc.path, startLine, endLine, side,
    codeSnippet: doc.kind === "pr" ? "" : snippet,
    kind: doc.kind, ...ext,
    comparison: doc.kind === "diff" ? doc.comparison : undefined,
    scope: doc.kind === "diff" ? doc.scope : undefined,
    commitOid: doc.kind === "commit" ? doc.oid : undefined,
    pullLabel, revision: ready.revision,
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
  readonly machineId?: string | undefined;
  readonly contextLabel?: string | undefined;
  readonly reviewContextId?: string | undefined;
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
    machineId: input.machineId,
    contextLabel: input.contextLabel,
    reviewContextId: input.reviewContextId,
    comparison: input.entry.comparison,
    scope: input.entry.scope,
    revision: input.revision,
  };
}
