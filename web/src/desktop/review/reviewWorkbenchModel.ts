import type { EditorView } from "@codemirror/view";
import { reviewEntryKey, diffHunkLines } from "../../mobile/review/diffNavigationModel";
import type { GitReviewEntry } from "../../mobile/review/gitReviewModel";
import { reviewSelectionMessage } from "../../mobile/review/reviewLineSelection";
import { editorSelection } from "./reviewEditorSelection";
import type { DesktopReviewDocument } from "./desktopReviewTabs";
import { adjacentIndex, adjacentLine } from "./reviewDocumentKeys";
import { pullFile } from "./pullPatches";

/** The change-list key of an open diff, as reviewed marks use it. */
export function documentEntryKey(document: DesktopReviewDocument | undefined): string | undefined {
  return document?.kind === "diff"
    ? reviewEntryKey(document.path, document.scope, document.comparison)
    : undefined;
}

export function entryDocument(entry: GitReviewEntry): DesktopReviewDocument {
  return {
    kind: "diff",
    path: entry.change.path,
    scope: entry.scope,
    ...(entry.comparison ? { comparison: entry.comparison } : {}),
  };
}

/** Where `]c`/`[c` lands: the line after the next (or previous) hunk header. */
export function hunkTarget(text: string, cursorLine: number, offset: 1 | -1): number | undefined {
  const headers = diffHunkLines(text);
  const header = offset > 0
    ? adjacentLine(headers, cursorLine, 1)
    : adjacentLine(headers, cursorLine - 1, -1);
  return header === undefined ? undefined : header + 1;
}

/** Which hunk the cursor reads, one-based, and how many there are. */
export function hunkPosition(text: string, cursorLine: number): { index: number; total: number } {
  const headers = diffHunkLines(text);
  return { index: headers.filter((line) => line <= cursorLine).length, total: headers.length };
}

/** The queue entry `]f`/`]u` opens next, or undefined at an end. */
export function adjacentEntry(
  queue: readonly GitReviewEntry[],
  current: string | undefined,
  offset: 1 | -1,
  accept: (entry: GitReviewEntry) => boolean = () => true,
): GitReviewEntry | undefined {
  const index = queue.findIndex((entry) =>
    reviewEntryKey(entry.change.path, entry.scope, entry.comparison) === current
  );
  const next = adjacentIndex(queue.length, index, offset, (at) => accept(queue[at]!));
  return next === undefined ? undefined : queue[next];
}

/** The Composer text for the Document's selection, naming a root that is
 *  not the Agent's own worktree so a same-named path cannot be confused, and
 *  the commit a History file belongs to. */
export function documentRemark(
  view: EditorView,
  document: DesktopReviewDocument,
  root: string | undefined,
): string {
  const message = reviewSelectionMessage({
    path: document.path,
    text: view.state.doc.toString(),
    kind: document.kind === "source" ? "source" : "diff",
    selection: editorSelection(view),
    comparison: document.kind === "diff" ? document.comparison : undefined,
  });
  const commit = document.kind === "commit"
    ? `In commit ${document.oid.slice(0, 10)}:\n`
    : document.kind === "pr"
    ? `In pull request ${pullRepository(document)}#${document.number}:\n`
    : "";
  return `${root ? `In \`${root}\`:\n` : ""}${commit}${message}`;
}

function pullRepository(document: Extract<DesktopReviewDocument, { kind: "pr" }>): string {
  const binding = pullFile(document.repositoryId, document.number, document.revision, document.path)?.binding;
  return binding ? `${binding.owner}/${binding.repository}` : document.repositoryId;
}

export function pathRemark(path: string, root: string | undefined): string {
  return `${root ? `In \`${root}\`: ` : ""}\`${path}\`\n\n`;
}
