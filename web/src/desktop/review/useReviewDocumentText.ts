import { useEffect, useState } from "react";
import {
  CodeApiError,
  fetchCodeBlob,
  fetchCodeDiffPage,
  fetchCodeFile,
  fetchCodeFilePage,
  fetchGitCommitDiff,
} from "../../mobile/review/codeApi";
import { loadCodeDiff } from "../../mobile/review/diffCache";
import type { ReviewSettings } from "../../mobile/review/reviewSettingsModel";
import { type DesktopReviewDocument, reviewDocumentKey } from "./desktopReviewTabs";
import { pullFile } from "./pullPatches";
import { diffBaseOid, expandDiffContext } from "./diffContextExpansion";
import type { ReviewDiffContext } from "./reviewDiffLayout";

/** Pages read for one document before it is shown as cut short: 64 diff
 *  pages are the server's 16 MiB diff bound; 16 file windows are 4 MiB. */
const DIFF_PAGES = 64;
const FILE_PAGES = 16;

export interface ReviewDocumentText {
  readonly key: string;
  readonly text: string;
  readonly revision: string | undefined;
  readonly added: number | undefined;
  readonly removed: number | undefined;
  /** More exists than was read (a bound, or the server's own cut). */
  readonly truncated: boolean;
  /** A diff asked for the whole file: `file` when its gaps were filled,
   *  `limited` when only the patch could be shown. */
  readonly context?: "file" | "limited" | undefined;
}

/**
 * The diff with the whole file as context (`␣VZ`), from the exact old side
 * its `index` line names. An ignore-whitespace or cut-short patch, a new or
 * binary file, and a Machine without the blob route keep the patch.
 */
async function withFileContext(
  sessionId: string,
  value: ReviewDocumentText,
  exact: boolean,
  signal: AbortSignal,
): Promise<ReviewDocumentText> {
  const oid = exact && !value.truncated ? diffBaseOid(value.text) : undefined;
  if (!oid) return { ...value, context: "limited" };
  try {
    const base = await fetchCodeBlob(sessionId, oid, signal);
    const text = expandDiffContext(value.text, base.text);
    return text ? { ...value, text, context: "file" } : { ...value, context: "limited" };
  } catch (reason) {
    if (reason instanceof DOMException && reason.name === "AbortError") throw reason;
    return { ...value, context: "limited" };
  }
}

export type ReviewDocumentLoad =
  | { readonly state: "loading" }
  | { readonly state: "ready"; readonly value: ReviewDocumentText }
  | { readonly state: "error"; readonly message: string };

function failure(reason: unknown): string {
  if (reason instanceof PullFileGone) return reason.message;
  if (reason instanceof CodeApiError) {
    if (reason.status === 404) return "This file is not in the worktree any more.";
    if (reason.status === 409 || reason.status === 410) return "The file changed while it was read. Refreshing…";
    if (reason.status === 415) return "This file is not text.";
    return `The file could not be read (${String(reason.status)}).`;
  }
  return "The file could not be read.";
}

async function readDiff(
  sessionId: string,
  document: Extract<DesktopReviewDocument, { kind: "diff" }>,
  settings: ReviewSettings,
  signal: AbortSignal,
): Promise<ReviewDocumentText> {
  const first = await loadCodeDiff(
    sessionId,
    document.path,
    settings.contextLines,
    settings.showWhitespaceChanges,
    document.scope,
    document.comparison,
    signal,
  );
  let text = first.text;
  let cursor = first.nextCursor;
  // Pages of one snapshot only: a page of another revision would splice two
  // different diffs together (13-code-review).
  for (let page = 1; cursor && page < DIFF_PAGES; page++) {
    const next = await fetchCodeDiffPage(sessionId, cursor, signal);
    if (next.revision !== first.revision) throw new CodeApiError(409);
    text += next.text;
    cursor = next.nextCursor;
  }
  return {
    key: reviewDocumentKey(document),
    text,
    revision: first.revision,
    added: first.added,
    removed: first.removed,
    truncated: Boolean(cursor) || first.truncated,
  };
}

async function readSource(
  sessionId: string,
  document: Extract<DesktopReviewDocument, { kind: "source" }>,
  signal: AbortSignal,
): Promise<ReviewDocumentText> {
  const first = await fetchCodeFile(sessionId, document.path, signal);
  let text = first.text;
  let cursor = first.nextCursor;
  for (let page = 1; cursor && page < FILE_PAGES; page++) {
    const next = await fetchCodeFilePage(sessionId, document.path, cursor, signal);
    text += next.text;
    cursor = next.nextCursor;
  }
  return {
    key: reviewDocumentKey(document),
    text,
    revision: first.revision,
    added: undefined,
    removed: undefined,
    truncated: Boolean(cursor) || (first.truncated && !first.nextCursor),
  };
}

async function readCommit(
  sessionId: string,
  document: Extract<DesktopReviewDocument, { kind: "commit" }>,
  signal: AbortSignal,
): Promise<ReviewDocumentText> {
  const diff = await fetchGitCommitDiff(sessionId, document.oid, document.path, signal);
  return {
    key: reviewDocumentKey(document),
    text: diff.text,
    revision: diff.revision,
    added: diff.added,
    removed: diff.removed,
    truncated: diff.truncated,
  };
}

/** A PR file's patch from memory; a reload forgets it (pullPatches.ts). */
function readPull(document: Extract<DesktopReviewDocument, { kind: "pr" }>): Promise<ReviewDocumentText> {
  const ref = pullFile(document.repositoryId, document.number, document.revision, document.path);
  if (!ref) return Promise.reject(new PullFileGone("Reopen this file from the Pull request tab (␣VG)."));
  if (ref.file.patch === null) {
    return Promise.reject(new PullFileGone("No text patch is available: a binary file, or a rename without text changes."));
  }
  return Promise.resolve({
    key: reviewDocumentKey(document),
    text: ref.file.patch,
    revision: document.revision,
    added: ref.file.additions,
    removed: ref.file.deletions,
    truncated: ref.file.limited,
  });
}

class PullFileGone extends Error {}

/** The text of the open Review document, reread when the worktree changes. */
export function useReviewDocumentText(
  sessionId: string | undefined,
  document: DesktopReviewDocument | undefined,
  settings: ReviewSettings,
  dataRevision: number,
  diffContext: ReviewDiffContext = "patch",
): ReviewDocumentLoad | undefined {
  const [load, setLoad] = useState<{ key: string; value: ReviewDocumentLoad }>();
  const key = document && sessionId ? `${sessionId}\0${reviewDocumentKey(document)}` : undefined;
  useEffect(() => {
    if (!sessionId || !document || !key) return undefined;
    const controller = new AbortController();
    // Keep showing the previous text of the same document while it refreshes.
    setLoad((previous) => previous?.key === key ? previous : { key, value: { state: "loading" } });
    const patch = document.kind === "diff"
      ? readDiff(sessionId, document, settings, controller.signal)
      : document.kind === "pr"
      ? readPull(document)
      : document.kind === "commit"
      ? readCommit(sessionId, document, controller.signal)
      : readSource(sessionId, document, controller.signal);
    // A PR's blobs live on the host, not in this repository.
    const expand = diffContext === "file" && (document.kind === "diff" || document.kind === "commit");
    const exact = document.kind === "commit" || settings.showWhitespaceChanges;
    const read = expand
      ? patch.then((value) => withFileContext(sessionId, value, exact, controller.signal))
      : patch;
    void read.then(
      (value) => {
        if (!controller.signal.aborted) setLoad({ key, value: { state: "ready", value } });
      },
      (reason: unknown) => {
        if (controller.signal.aborted) return;
        if (reason instanceof DOMException && reason.name === "AbortError") return;
        setLoad({ key, value: { state: "error", message: failure(reason) } });
      },
    );
    return () => controller.abort();
    // `document` is identified by `key`; settings that change the diff refetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, dataRevision, settings.contextLines, settings.showWhitespaceChanges, diffContext]);
  return load?.key === key ? load?.value : undefined;
}
