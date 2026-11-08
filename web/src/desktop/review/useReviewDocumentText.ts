import { useEffect, useState } from "react";
import {
  CodeApiError,
  fetchCodeDiffPage,
  fetchCodeFile,
  fetchCodeFilePage,
} from "../../mobile/review/codeApi";
import { loadCodeDiff } from "../../mobile/review/diffCache";
import type { ReviewSettings } from "../../mobile/review/reviewSettingsModel";
import { type DesktopReviewDocument, reviewDocumentKey } from "./desktopReviewTabs";

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
}

export type ReviewDocumentLoad =
  | { readonly state: "loading" }
  | { readonly state: "ready"; readonly value: ReviewDocumentText }
  | { readonly state: "error"; readonly message: string };

function failure(reason: unknown): string {
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

/** The text of the open Review document, reread when the worktree changes. */
export function useReviewDocumentText(
  sessionId: string | undefined,
  document: DesktopReviewDocument | undefined,
  settings: ReviewSettings,
  dataRevision: number,
): ReviewDocumentLoad | undefined {
  const [load, setLoad] = useState<{ key: string; value: ReviewDocumentLoad }>();
  const key = document && sessionId ? `${sessionId}\0${reviewDocumentKey(document)}` : undefined;
  useEffect(() => {
    if (!sessionId || !document || !key) return undefined;
    const controller = new AbortController();
    // Keep showing the previous text of the same document while it refreshes.
    setLoad((previous) => previous?.key === key ? previous : { key, value: { state: "loading" } });
    const read = document.kind === "diff"
      ? readDiff(sessionId, document, settings, controller.signal)
      : readSource(sessionId, document, controller.signal);
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
  }, [key, dataRevision, settings.contextLines, settings.showWhitespaceChanges]);
  return load?.key === key ? load?.value : undefined;
}
