import type { EditorView } from "@codemirror/view";
import { useCallback, useMemo, useRef, useState } from "react";
import { mutateMobileReview, notify } from "../../store";
import { appendDraftText } from "../../draftStore";
import { currentReviewMarks, revisionMatches } from "../../mobile/review/reviewProgress";
import type { GitReviewEntry } from "../../mobile/review/gitReviewModel";
import { useReviewSettings } from "../../mobile/review/reviewSettings";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { entryKey } from "./ReviewChangesList";
import {
  activeReviewDocument,
  closeReviewTab,
  cycleReviewTab,
  keepReviewTab,
  nthReviewTab,
  openReviewDocument,
  reviewDocumentKey,
} from "./desktopReviewTabs";
import { revealEditorLine } from "./reviewEditorSelection";
import type { ReviewDocumentAction } from "./reviewDocumentKeys";
import {
  adjacentEntry,
  documentEntryKey,
  documentRemark,
  entryDocument,
  hunkPosition,
  hunkTarget,
  pathRemark,
} from "./reviewWorkbenchModel";
import { useDesktopReviewChanges } from "./useDesktopReviewChanges";
import { useDesktopReviewContext } from "./useDesktopReviewContext";
import { useReviewDocumentText } from "./useReviewDocumentText";
import { useReviewTabsState } from "./useReviewTabsState";

export const REVIEW_DOCUMENT_REGION = "review.document";

/** Everything the Desktop Review workbench reads and does, without layout. */
export function useReviewWorkbench(active: boolean) {
  const workspace = useDesktopWorkspace();
  const context = useDesktopReviewContext();
  const target = context.target;
  const settings = useReviewSettings();
  const changes = useDesktopReviewChanges(target?.sessionId, context.stateKey, context.synced.comparison, active);
  const [tabs, updateTabs] = useReviewTabsState(context.stateKey);
  const document = activeReviewDocument(tabs);
  const load = useReviewDocumentText(target?.sessionId, document, settings, changes.dataRevision);
  const editor = useRef<EditorView | null>(null);
  const positions = useRef(new Map<string, number>());
  const [cursorLine, setCursorLine] = useState(1);
  const ready = load?.state === "ready" ? load.value : undefined;
  const reviewed = useMemo(() => currentReviewMarks(
    context.synced.progress,
    changes.queue.map((entry) => ({ key: entryKey(entry), fingerprint: entry.change.fingerprint })),
  ), [changes.queue, context.synced.progress]);
  const openKey = documentEntryKey(document);
  const openEntry = changes.queue.find((entry) => entryKey(entry) === openKey);
  const openMark = openEntry?.change.fingerprint ?? ready?.revision;
  const root = target && !target.own ? target.cwd : undefined;

  const focusDocument = useCallback((): void => {
    requestAnimationFrame(() => workspace.focusRegion(REVIEW_DOCUMENT_REGION));
  }, [workspace]);
  const open = useCallback((entry: GitReviewEntry): void => {
    updateTabs((state) => openReviewDocument(state, entryDocument(entry)));
    focusDocument();
  }, [focusDocument, updateTabs]);
  const openSource = useCallback((path: string): void => {
    updateTabs((state) => openReviewDocument(state, { kind: "source", path }));
    focusDocument();
  }, [focusDocument, updateTabs]);
  const openCommit = useCallback((oid: string, path: string): void => {
    updateTabs((state) => openReviewDocument(state, { kind: "commit", oid, path }));
    focusDocument();
  }, [focusDocument, updateTabs]);
  const markEntry = useCallback((key: string, mark: string | undefined): void => {
    if (!context.stateKey || !mark) {
      notify("Open the diff to mark it reviewed", "info");
      return;
    }
    const done = revisionMatches(context.synced.progress, key, mark);
    mutateMobileReview(context.stateKey, "markReviewed", { key, revision: done ? null : mark });
  }, [context.stateKey, context.synced.progress]);
  const send = useCallback((stay: boolean, text?: string): void => {
    const agent = context.agentSessionId;
    const view = editor.current;
    const message = text ?? (view && document ? documentRemark(view, document, root) : undefined);
    if (!agent || !message) return;
    appendDraftText(agent, message);
    // After Vim has left Visual mode and settled its own focus.
    if (!stay) requestAnimationFrame(() => workspace.focusRegion("prompt.composer"));
    else notify("Added to the Prompt", "info");
  }, [context.agentSessionId, document, root, workspace]);

  const onAction = useCallback((action: ReviewDocumentAction, view: EditorView): void => {
    switch (action.kind) {
      case "hunk": {
        const line = hunkTarget(view.state.doc.toString(), cursorLine, action.offset);
        if (line === undefined) notify(action.offset > 0 ? "Last hunk" : "First hunk", "info");
        else revealEditorLine(view, line);
        return;
      }
      case "file":
      case "unreviewed": {
        const next = adjacentEntry(changes.queue, openKey, action.offset, (entry) =>
          action.kind === "file" || !reviewed.has(entryKey(entry)));
        if (next) open(next);
        else notify(action.kind === "file" ? "No more changes" : "No more unreviewed changes", "info");
        return;
      }
      case "tab":
        updateTabs((state) => cycleReviewTab(state, action.offset));
        return;
      case "nthTab":
        updateTabs((state) => nthReviewTab(state, action.n));
        return;
      case "send":
        send(false);
    }
  }, [changes.queue, cursorLine, open, openKey, reviewed, send, updateTabs]);

  const onCursorLine = useCallback((line: number): void => {
    setCursorLine(line);
    if (document) positions.current.set(reviewDocumentKey(document), line);
  }, [document]);

  return {
    workspace,
    context,
    target,
    settings,
    changes,
    tabs,
    updateTabs,
    document,
    load,
    editor,
    reviewed,
    openKey,
    restoreLine: document ? positions.current.get(reviewDocumentKey(document)) : undefined,
    hunk: document?.kind === "diff" && ready ? hunkPosition(ready.text, cursorLine) : undefined,
    openReviewed: openKey && openMark ? revisionMatches(context.synced.progress, openKey, openMark) : undefined,
    root,
    open,
    openSource,
    openCommit,
    markOpen: () => openKey && markEntry(openKey, openMark),
    markEntry,
    send,
    onAction,
    onCursorLine,
    closeTab: (key?: string) => updateTabs((state) => closeReviewTab(state, key ?? state.active)),
    keepTab: (key?: string) => updateTabs((state) => keepReviewTab(state, key ?? state.active)),
    pathRemark: (path: string) => pathRemark(path, root),
  };
}

export type ReviewWorkbench = ReturnType<typeof useReviewWorkbench>;
