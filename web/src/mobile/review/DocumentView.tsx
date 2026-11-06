import { KeyboardArrowDown, KeyboardArrowUp, Refresh } from "@mui/icons-material";
import { Box, Button, ButtonBase, Chip, CircularProgress, Popover, Stack, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { navigationHaptic } from "../../haptic";
import { Markdown } from "../../Markdown";
import { Sheet } from "../../Sheet";
import { mutateMobileReview as mutateReviewState } from "../../store";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";
import { CodeApiError, type CodeHover, type CodeLocation, type CodeNavigationKind, fetchCodeDiffPage, fetchCodeFile, fetchCodeFilePage, fetchCodeHover, fetchCodeNavigation, isTransientCodeApiStatus, shouldCloseUnavailableSource } from "./codeApi";
import { loadCodeDiff } from "./diffCache";
import { diffHunkLines } from "./diffNavigationModel";
import type { CodeInspectCandidate, CodeRevealRange } from "./CodeViewer";
import { ReviewOutline } from "./ReviewOutline";
import { ReviewNavigation } from "./ReviewNavigation";
import { type ReviewBufferMode } from "./ownedReviewBuffer";
import { reviewDisplayText, useOwnedReviewBuffer } from "./useOwnedReviewBuffer";
import { ReviewCodeStatus } from "./ReviewCodeStatus";
import { ReviewCheckStatus } from "./ReviewCheckStatus";
import { markdownPreviewSx } from "./reviewMarkdownPreview";
import { ReviewDiffCodeStatus } from "./ReviewDiffCodeStatus";
import { ReviewDocumentControls } from "./ReviewDocumentControls";
import { useOwnedReviewDiff } from "./useOwnedReviewDiff";
import { captureTextScrollAnchor, DOCUMENT_REFRESH_RESUME_GRACE_MS, documentRefreshDecision, restoreTextScrollAnchor, type TextScrollAnchor } from "./documentRefreshModel";
import { ReviewMediaPreview } from "./ReviewMediaPreview";
import { MermaidDiagram } from "./MermaidDiagram";
import { isReviewMediaPath, reviewPreviewKind } from "./reviewPreview";
import { readReviewSettings, subscribeReviewSettings, useReviewSettings } from "./reviewSettings";
import { ReviewSelectionBar } from "./ReviewSelectionBar";
import { nextReviewLineSelection, type ReviewLineSelection, type ReviewQuestion, reviewSelectionMessage } from "./reviewLineSelection";
import { presentHoverBlock } from "./symbolPresentation";
import type { CodeDiffScope } from "./codeApi";
import type { CodeLanguage } from "./codeApi";
import type { GitReviewEntry } from "./gitReviewModel";
import { restoreReviewScrollTop } from "./reviewScrollPosition";
import { HintTooltip } from "../../HintTooltip";

export const SYMBOL_NAVIGATION_KINDS = [
  "definition",
  "declaration",
  "typeDefinition",
  "implementation",
  "references",
] as const satisfies readonly CodeNavigationKind[];

export type SymbolNavigationAvailability = Record<
  CodeNavigationKind,
  "checking" | "available" | "unavailable"
>;

export function checkingSymbolNavigation(): SymbolNavigationAvailability {
  return Object.fromEntries(
    SYMBOL_NAVIGATION_KINDS.map((kind) => [kind, "checking"]),
  ) as SymbolNavigationAvailability;
}

export const CodeViewer = lazy(() => import("./CheckedCodeViewer"));

export type ReviewTarget =
  | { kind: "changes" }
  | {
    kind: "diff";
    path: string;
    scope: CodeDiffScope;
    comparison?: string;
    queue: GitReviewEntry[];
  }
  | {
    kind: "source";
    path: string;
    revealLine?: number;
    revealRange?: CodeRevealRange;
    revealRequestId?: number;
  };

export type SymbolPoint = { row: number; column: number };

export type LoadedReviewDocument =
  | Awaited<ReturnType<typeof fetchCodeFile>>
  | Awaited<ReturnType<typeof loadCodeDiff>>;

export type PendingDocumentUpdate =
  | { kind: "content"; result: LoadedReviewDocument }
  | { kind: "removed" };

export function usePageVisible(): boolean {
  const [visible, setVisible] = useState(
    () => document.visibilityState === "visible",
  );
  useEffect(() => {
    const update = (): void =>
      setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return visible;
}

export type SymbolRestoreRequest = SymbolPoint & { path: string; id: number };

// Exported for the synthetic-HTTP browser conformance fixture only.
export function DocumentView({
  sessionId,
  reviewStateId = sessionId,
  target,
  onRevision,
  markdownPreview,
  languageData,
  onNavigate,
  restoreSymbol,
  closeSymbolRequest,
  onRestoreSymbolConsumed,
  onSymbolOpenChange,
  onVisibleSourceLine,
  onBufferUnavailable,
  readScrollPosition,
  rememberScrollPosition,
  onMarkdownLink,
  previewAnchor,
  bufferMode,
  dataRevision,
  active,
  outlineOpen,
  onOutlineClose,
  outlineLine,
  onOutlineSelect,
  onSendSelection,
  onExplainSelection,
}: {
  sessionId: string;
  reviewStateId?: string;
  /** Hand selected lines to the Agent's Composer; absent without an Agent. */
  onSendSelection?: ((message: string) => void) | undefined;
  /** Ask a fork of the Agent about selected lines; absent where none exists. */
  onExplainSelection?: ((path: string, message: string, kind: ReviewQuestion) => Promise<void>) | undefined;
  target: Exclude<ReviewTarget, { kind: "changes" }>;
  onRevision: (revision: string | undefined) => void;
  markdownPreview: boolean;
  languageData?: CodeLanguage | undefined;
  onNavigate: (
    location: CodeLocation,
    origin: { row: number; column: number },
  ) => void;
  restoreSymbol?: SymbolRestoreRequest | undefined;
  closeSymbolRequest: number;
  onRestoreSymbolConsumed: (id: number) => void;
  onSymbolOpenChange: (point: SymbolPoint | undefined) => void;
  onVisibleSourceLine: (line: number) => void;
  onBufferUnavailable: () => void;
  readScrollPosition: (key: string) => unknown;
  rememberScrollPosition: (key: string, top: number) => void;
  /** Claim a rendered-Markdown link. True when the app navigated instead of
   *  letting it open as a web address. */
  onMarkdownLink: (href: string) => boolean;
  /** A heading to land on once THIS document is rendered — either a same-file
   *  `#hash` or the tail of a `other.md#hash` that is still loading. */
  previewAnchor?: { path: string; hash: string; id: number } | undefined;
  bufferMode: ReviewBufferMode | undefined;
  dataRevision: number;
  /** Review is the foreground pane. A reader in front of the document is
   *  asked before its text changes; otherwise refreshes apply in place. */
  active: boolean;
  outlineOpen: boolean;
  onOutlineClose: () => void;
  outlineLine: number | undefined;
  onOutlineSelect: (line: number) => void;
}): React.JSX.Element {
  const surface = useSurfaceProfile();
  const [lineSelection, setLineSelection] = useState<ReviewLineSelection>();
  const selectionMessage = (selection: ReviewLineSelection): string =>
    reviewSelectionMessage({
      path: target.path,
      text: displayText,
      kind: target.kind,
      selection,
      comparison: target.kind === "diff" ? target.comparison : undefined,
    });
  const toggleSelectedLine = useCallback((line: number): void => {
    setLineSelection((current) => nextReviewLineSelection(current, line));
  }, []);
  const mutateMobileReview: typeof mutateReviewState = useCallback((_context, name, args) => mutateReviewState(reviewStateId, name, args), [reviewStateId]);
  const settings = useReviewSettings();
  const previewKind = reviewPreviewKind(target.path);
  const mediaPreview = isReviewMediaPath(target.path);
  const [text, setText] = useState("");
  const [truncated, setTruncated] = useState(false);
  const [limited, setLimited] = useState(false);
  const [nextCursor, setNextCursor] = useState<string>();
  const [revision, setRevision] = useState<string>();
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadMoreError, setLoadMoreError] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [pendingUpdate, setPendingUpdate] = useState<PendingDocumentUpdate>();
  const [refreshRetry, setRefreshRetry] = useState(0);
  const pageVisible = usePageVisible();
  const reading = active && pageVisible;
  const pageController = useRef<AbortController | undefined>(undefined);
  const [counts, setCounts] = useState<{ added: number; removed: number }>();
  const [loading, setLoading] = useState(true);
  const [loadedPath, setLoadedPath] = useState<string>();
  const [error, setError] = useState<number | "network">();
  const [hunkIndex, setHunkIndex] = useState(0);
  const [hover, setHover] = useState<CodeHover>();
  const [hoverOpen, setHoverOpen] = useState(false);
  const [hoverLoading, setHoverLoading] = useState(false);
  const [hoverError, setHoverError] = useState(false);
  const [inspectTarget, setInspectTarget] = useState<
    { row: number; column: number } | undefined
  >();
  const [navigation, setNavigation] = useState<CodeLocation[]>([]);
  const [navigationAvailability, setNavigationAvailability] = useState(
    checkingSymbolNavigation,
  );
  const [navigationResults, setNavigationResults] = useState<
    Partial<Record<CodeNavigationKind, CodeLocation[]>>
  >({});
  const [inspectCandidates, setInspectCandidates] = useState<
    CodeInspectCandidate[]
  >([]);
  const [inspectCandidatesExpanded, setInspectCandidatesExpanded] = useState(
    false,
  );
  const [inspectAnchor, setInspectAnchor] = useState<
    { top: number; left: number } | undefined
  >();
  const hoverController = useRef<AbortController | undefined>(undefined);
  const navigationController = useRef<AbortController | undefined>(undefined);
  const usingOwned = bufferMode === "owned";
  const isPatch = target.kind === "diff" && previewKind !== "mermaid";
  const displayText = useMemo(
    () => usingOwned ? reviewDisplayText(text) : text,
    [usingOwned, text],
  );
  const completeText = !loading && !error && loadedPath === target.path &&
      !truncated && !limited && !nextCursor
    ? displayText
    : undefined;
  const diff = useOwnedReviewDiff(
    sessionId,
    target.path,
    usingOwned && isPatch && !mediaPreview,
    target.kind === "diff" ? target.scope : "unstaged",
    completeText,
  );
  const owned = useOwnedReviewBuffer(
    sessionId,
    target.path,
    usingOwned && !mediaPreview && (target.kind === "source" ||
      (isPatch && target.scope !== "staged")),
    isPatch ? diff.projection?.source : completeText,
    undefined,
    isPatch ? diff.projection : undefined,
  );
  const fileRetry = useRef<{ key: string; count: number }>({
    key: "",
    count: 0,
  });
  const positionTimer = useRef<number | undefined>(undefined);
  const pendingPosition = useRef<number | undefined>(undefined);
  const hunks = target.kind === "diff" ? diffHunkLines(text) : [];
  const tabKey = target.kind === "source"
    ? `source:${target.path}`
    : `diff:${target.comparison ?? target.scope}:${target.path}`;
  const scrollKey = `${sessionId}:${tabKey}`;
  const outerScrollKey = `${scrollKey}:page`;
  const editorScrollKey = `${scrollKey}:editor`;
  const outerScrollRef = useRef<HTMLDivElement>(null);
  const outerRestoreFrame = useRef(0);
  const appliedOuterRestore = useRef<string | undefined>(undefined);
  const outerScrollable = markdownPreview || previewKind === "mermaid" ||
    mediaPreview || settings.softWrap;
  const outerAnchor = useRef<TextScrollAnchor | undefined>(undefined);
  // Toggling Markdown wrap reflows every code block and table. The store
  // notifies synchronously, before React replaces the old layout, so the
  // reading anchor is captured here and restored after the reflow below.
  const markdownPreviewRef = useRef(markdownPreview);
  markdownPreviewRef.current = markdownPreview;
  useEffect(() => {
    let wrap = readReviewSettings().markdownSoftWrap;
    return subscribeReviewSettings(() => {
      const next = readReviewSettings().markdownSoftWrap;
      if (next === wrap) return;
      wrap = next;
      const scroller = outerScrollRef.current;
      if (!markdownPreviewRef.current || !scroller) return;
      outerAnchor.current = captureTextScrollAnchor(scroller);
    });
  }, []);
  const documentState = useRef({
    loading,
    error,
    loadedPath,
    revision,
    text,
  });
  documentState.current = { loading, error, loadedPath, revision, text };
  const readingRef = useRef(reading);
  const autoApplyUntil = useRef(
    performance.now() + DOCUMENT_REFRESH_RESUME_GRACE_MS,
  );
  const pendingRef = useRef(pendingUpdate);
  pendingRef.current = pendingUpdate;
  // A `#heading` may arrive before the file it belongs to has loaded (a
  // cross-file `other.md#section`), so the scroll waits for THIS document to be
  // the one asked for and for its text to be on screen. Consumed by id, so the
  // same anchor tapped twice still scrolls.
  const appliedAnchor = useRef(0);
  useLayoutEffect(() => {
    if (
      !previewAnchor || !markdownPreview ||
      previewAnchor.path !== target.path ||
      loadedPath !== target.path || appliedAnchor.current === previewAnchor.id
    ) return undefined;
    const frame = globalThis.requestAnimationFrame(() => {
      const scroller = outerScrollRef.current;
      const heading = scroller?.querySelector<HTMLElement>(
        `[id="${CSS.escape(previewAnchor.hash)}"]`,
      );
      if (!scroller || !heading) return;
      appliedAnchor.current = previewAnchor.id;
      // Position by measurement rather than scrollIntoView: the preview lives
      // inside the peek compositor, and scrollIntoView on iOS may scroll an
      // ancestor instead, dragging the whole workspace sideways.
      const top = scroller.scrollTop +
        heading.getBoundingClientRect().top -
        scroller.getBoundingClientRect().top - 12;
      scroller.scrollTop = Math.max(0, top);
      rememberScrollPosition(outerScrollKey, scroller.scrollTop);
    });
    return () => globalThis.cancelAnimationFrame(frame);
  }, [
    loadedPath,
    markdownPreview,
    outerScrollKey,
    previewAnchor,
    rememberScrollPosition,
    target.path,
    text,
  ]);

  useLayoutEffect(() => {
    if (
      !outerScrollable || loading || error || loadedPath !== target.path ||
      appliedOuterRestore.current === outerScrollKey
    ) return undefined;
    globalThis.cancelAnimationFrame(outerRestoreFrame.current);
    outerRestoreFrame.current = globalThis.requestAnimationFrame(() => {
      const element = outerScrollRef.current;
      if (!element || appliedOuterRestore.current === outerScrollKey) return;
      appliedOuterRestore.current = outerScrollKey;
      const restored = restoreReviewScrollTop(
        element,
        readScrollPosition(outerScrollKey),
      );
      rememberScrollPosition(outerScrollKey, restored);
    });
    return () => globalThis.cancelAnimationFrame(outerRestoreFrame.current);
  }, [
    error,
    loadedPath,
    loading,
    outerScrollKey,
    outerScrollable,
    readScrollPosition,
    rememberScrollPosition,
    target.path,
    text,
  ]);

  const persistVisibleLine = useCallback((line: number): void => {
    if (target.kind !== "source") return;
    onVisibleSourceLine(line);
    pendingPosition.current = line;
    if (positionTimer.current !== undefined) {
      globalThis.clearTimeout(positionTimer.current);
    }
    positionTimer.current = globalThis.setTimeout(() => {
      const pendingLine = pendingPosition.current;
      positionTimer.current = undefined;
      pendingPosition.current = undefined;
      if (pendingLine === undefined) return;
      mutateMobileReview(sessionId, "setPosition", {
        path: target.path,
        line: pendingLine,
        revision: revision ?? null,
      });
    }, 500);
  }, [onVisibleSourceLine, revision, sessionId, target]);

  useEffect(() => {
    return () => {
      if (positionTimer.current !== undefined) {
        globalThis.clearTimeout(positionTimer.current);
        positionTimer.current = undefined;
      }
      const line = pendingPosition.current;
      pendingPosition.current = undefined;
      if (target.kind === "source" && line !== undefined) {
        mutateMobileReview(sessionId, "setPosition", {
          path: target.path,
          line,
          revision: revision ?? null,
        });
      }
    };
  }, [revision, sessionId, target]);

  const inspectPoint = useCallback((
    point: {
      row: number;
      column: number;
    },
    candidates: CodeInspectCandidate[] = [],
    autoFallback = true,
  ): void => {
    if (!bufferMode || bufferMode === "unavailable") return;
    if (
      target.kind === "diff" &&
      (usingOwned ? !diff.projection : target.scope !== "unstaged")
    ) return;
    setInspectTarget(point);
    setInspectCandidates(candidates);
    setNavigation([]);
    hoverController.current?.abort();
    const controller = new AbortController();
    hoverController.current = controller;
    setHover(undefined);
    setHoverError(false);
    setHoverLoading(true);
    setHoverOpen(true);
    const ordered = [
      point,
      ...(autoFallback
        ? candidates.filter((candidate) =>
          candidate.row !== point.row || candidate.column !== point.column
        )
        : []),
    ];
    void (async () => {
      for (const [index, candidate] of ordered.entries()) {
        const value = usingOwned
          ? await owned.hover(
            candidate.row,
            candidate.column,
            controller.signal,
          )
          : await fetchCodeHover(
            sessionId,
            target.path,
            candidate.row,
            candidate.column,
            controller.signal,
          );
        if (controller.signal.aborted) return;
        if (value.contents.length > 0 || index === ordered.length - 1) {
          setInspectTarget(candidate);
          setHover(value);
          setHoverError(false);
          return;
        }
      }
    })().catch(() => {
      if (!controller.signal.aborted) {
        setHover({ apiVersion: 1, path: target.path, contents: [] });
        setHoverError(true);
        if (!usingOwned) onBufferUnavailable();
      }
    }).finally(() => {
      if (!controller.signal.aborted) setHoverLoading(false);
    });
  }, [
    onBufferUnavailable,
    sessionId,
    target,
    usingOwned,
    owned.hover,
    bufferMode,
    diff.projection,
  ]);

  useLayoutEffect(() => {
    // Positional popovers are observations of ONE displayed snapshot.
    hoverController.current?.abort();
    navigationController.current?.abort();
    setHoverOpen(false);
    setHover(undefined);
    setNavigation([]);
  }, [sessionId, target.path, displayText, usingOwned, diff.projection]);

  const inspectCandidatesOrPoint = useCallback((
    candidates: CodeInspectCandidate[],
    anchor: { top: number; left: number },
  ): void => {
    setInspectAnchor(anchor);
    navigationHaptic();
    inspectPoint(candidates[0]!, candidates);
  }, [inspectPoint]);

  useEffect(() => {
    onSymbolOpenChange(hoverOpen ? inspectTarget : undefined);
  }, [hoverOpen, inspectTarget, onSymbolOpenChange]);

  useEffect(() => {
    if (!hoverOpen) setInspectCandidatesExpanded(false);
  }, [hoverOpen]);

  useEffect(() => {
    setHoverOpen(false);
    setHoverError(false);
    hoverController.current?.abort();
  }, [closeSymbolRequest]);

  useEffect(() => {
    if (
      !restoreSymbol ||
      target.path !== restoreSymbol.path ||
      loadedPath !== restoreSymbol.path
    ) return;
    inspectPoint(restoreSymbol);
    onRestoreSymbolConsumed(restoreSymbol.id);
  }, [
    inspectPoint,
    loadedPath,
    onRestoreSymbolConsumed,
    restoreSymbol,
    target.path,
  ]);

  useEffect(() => {
    navigationController.current?.abort();
    setNavigationResults({});
    setNavigationAvailability(
      usingOwned
        ? {
          definition: "unavailable",
          declaration: "unavailable",
          typeDefinition: "unavailable",
          implementation: "unavailable",
          references: "unavailable",
        }
        : checkingSymbolNavigation(),
    );
    if (
      usingOwned || !hoverOpen || !inspectTarget ||
      (target.kind === "diff" && target.scope !== "unstaged")
    ) return undefined;
    const controller = new AbortController();
    navigationController.current = controller;
    for (const kind of SYMBOL_NAVIGATION_KINDS) {
      void fetchCodeNavigation(
        sessionId,
        target.path,
        inspectTarget.row,
        inspectTarget.column,
        kind,
        controller.signal,
      ).then((result) => {
        if (controller.signal.aborted) return;
        setNavigationResults((current) => ({
          ...current,
          [kind]: result.locations,
        }));
        setNavigationAvailability((current) => ({
          ...current,
          [kind]: result.locations.length > 0 ? "available" : "unavailable",
        }));
      }).catch(() => {
        if (controller.signal.aborted) return;
        setNavigationAvailability((current) => ({
          ...current,
          [kind]: "unavailable",
        }));
      });
    }
    return () => controller.abort();
  }, [
    hoverOpen,
    inspectTarget?.column,
    inspectTarget?.row,
    sessionId,
    target.kind,
    target.path,
    target.kind === "diff" ? target.scope : undefined,
    usingOwned,
  ]);

  const navigate = useCallback((kind: CodeNavigationKind): void => {
    if (
      !inspectTarget ||
      (target.kind === "diff" && target.scope !== "unstaged") ||
      navigationAvailability[kind] !== "available"
    ) return;
    const locations = navigationResults[kind];
    if (!locations?.length) return;
    setNavigation([]);
    const onlyLocation = locations.length === 1 ? locations[0] : undefined;
    if (onlyLocation) {
      setHoverOpen(false);
      onNavigate(onlyLocation, inspectTarget);
    } else {
      setNavigation(locations);
    }
  }, [
    inspectTarget,
    navigationAvailability,
    navigationResults,
    onNavigate,
    target,
  ]);

  useEffect(() => {
    const controller = new AbortController();
    const diffTarget = target.kind === "diff" ? target : undefined;
    setPendingUpdate(undefined);
    setLoading(true);
    setError(undefined);
    setText("");
    setCounts(undefined);
    setLimited(false);
    setNextCursor(undefined);
    setRevision(undefined);
    setLoadedPath(undefined);
    setLoadMoreError(false);
    setHoverOpen(false);
    hoverController.current?.abort();
    onRevision(undefined);
    if (mediaPreview) {
      setLoadedPath(target.path);
      setLoading(false);
      return () => controller.abort();
    }
    // Image/SVG previews skip this fetch. Mermaid is text, but Git Changes
    // still opens a diff target — render the current file, not the patch.
    const request = diffTarget && previewKind !== "mermaid"
      ? loadCodeDiff(
        sessionId,
        diffTarget.path,
        settings.contextLines,
        settings.showWhitespaceChanges,
        diffTarget.scope,
        diffTarget.comparison,
        controller.signal,
      )
      : fetchCodeFile(sessionId, target.path, controller.signal);
    void request.then((result) => {
      if (controller.signal.aborted) return;
      fileRetry.current = {
        key: `${sessionId}:${target.kind}:${target.path}`,
        count: 0,
      };
      setText(result.text);
      setLoadedPath(target.path);
      setTruncated(result.truncated);
      setRevision(result.revision);
      setNextCursor(result.nextCursor);
      setLimited(result.limited ?? false);
      if ("added" in result) {
        if (!result.nextCursor) onRevision(result.revision);
        setCounts({ added: result.added, removed: result.removed });
        setHunkIndex(0);
        if (!diffTarget) return;
        const currentIndex = diffTarget.queue.findIndex((entry) =>
          entry.change.path === diffTarget.path &&
          entry.scope === diffTarget.scope &&
          entry.comparison === diffTarget.comparison
        );
        for (
          const entry of [
            diffTarget.queue[currentIndex - 1],
            diffTarget.queue[currentIndex + 1],
          ]
        ) {
          if (entry) {
            void loadCodeDiff(
              sessionId,
              entry.change.path,
              settings.contextLines,
              settings.showWhitespaceChanges,
              entry.scope,
              entry.comparison,
            ).catch(() => undefined);
          }
        }
      }
    }).catch((reason) => {
      if (controller.signal.aborted) return;
      if (reason instanceof DOMException && reason.name === "AbortError") {
        return;
      }
      const requestKey = `${sessionId}:${target.kind}:${target.path}`;
      if (fileRetry.current.key !== requestKey) {
        fileRetry.current = { key: requestKey, count: 0 };
      }
      const status = reason instanceof CodeApiError ? reason.status : undefined;
      if (shouldCloseUnavailableSource(target.kind, status)) {
        mutateMobileReview(sessionId, "close", { path: target.path });
        return;
      }
      const transient = isTransientCodeApiStatus(status);
      if (transient && fileRetry.current.count < 2) {
        const delay = fileRetry.current.count === 0 ? 400 : 1_200;
        fileRetry.current.count += 1;
        globalThis.setTimeout(() => {
          if (!controller.signal.aborted) {
            setReloadKey((value) => value + 1);
          }
        }, delay);
        return;
      }
      setError(status ?? "network");
    }).finally(() => {
      if (!controller.signal.aborted) setLoading(false);
    });
    return () => {
      controller.abort();
      pageController.current?.abort();
      hoverController.current?.abort();
    };
  }, [
    sessionId,
    settings.contextLines,
    settings.showWhitespaceChanges,
    target.kind,
    target.path,
    target.kind === "diff" ? target.scope : undefined,
    mediaPreview,
    onRevision,
    reloadKey,
  ]);

  // Replace the displayed snapshot without unmounting the viewer, keeping the
  // block at the top of the viewport where the reader left it.
  const applyInPlace = useRef<(result: LoadedReviewDocument) => void>(
    () => undefined,
  );
  applyInPlace.current = (result) => {
    const scroller = outerScrollRef.current;
    outerAnchor.current =
      outerScrollable && scroller && result.text !== documentState.current.text
        ? captureTextScrollAnchor(scroller)
        : undefined;
    pageController.current?.abort();
    setLoadingMore(false);
    setPendingUpdate(undefined);
    setText(result.text);
    setTruncated(result.truncated);
    setRevision(result.revision);
    setNextCursor(result.nextCursor);
    setLimited(result.limited ?? false);
    setLoadMoreError(false);
    if ("added" in result) {
      if (!result.nextCursor) onRevision(result.revision);
      setCounts({ added: result.added, removed: result.removed });
      const hunkCount = diffHunkLines(result.text).length;
      setHunkIndex((index) => Math.min(index, Math.max(0, hunkCount - 1)));
    }
  };

  useLayoutEffect(() => {
    const anchor = outerAnchor.current;
    const scroller = outerScrollRef.current;
    if (!anchor || !scroller) return undefined;
    outerAnchor.current = undefined;
    const restore = (): void => {
      if (restoreTextScrollAnchor(scroller, anchor)) {
        rememberScrollPosition(outerScrollKey, scroller.scrollTop);
      }
    };
    restore();
    // Highlighted code blocks and diagrams can settle one frame later.
    const frame = globalThis.requestAnimationFrame(restore);
    return () => globalThis.cancelAnimationFrame(frame);
  }, [outerScrollKey, rememberScrollPosition, settings.markdownSoftWrap, text]);

  // A worktree revision covers every file. Revalidate THIS document in the
  // background and only touch the screen when its own revision changed.
  const handledRefresh = useRef(`${dataRevision}:0`);
  const refreshRetries = useRef({ dataRevision, count: 0 });
  useEffect(() => {
    const key = `${dataRevision}:${refreshRetry}`;
    if (handledRefresh.current === key) return undefined;
    handledRefresh.current = key;
    if (refreshRetries.current.dataRevision !== dataRevision) {
      refreshRetries.current = { dataRevision, count: 0 };
    }
    const current = documentState.current;
    if (
      mediaPreview || current.loading || current.error !== undefined ||
      current.loadedPath !== target.path
    ) {
      setReloadKey((value) => value + 1);
      return undefined;
    }
    const controller = new AbortController();
    const diffTarget = target.kind === "diff" ? target : undefined;
    const request = diffTarget && previewKind !== "mermaid"
      ? loadCodeDiff(
        sessionId,
        diffTarget.path,
        settings.contextLines,
        settings.showWhitespaceChanges,
        diffTarget.scope,
        diffTarget.comparison,
        controller.signal,
      )
      : fetchCodeFile(sessionId, target.path, controller.signal);
    const unseen = (): boolean =>
      !readingRef.current || performance.now() < autoApplyUntil.current;
    void request.then((result) => {
      if (controller.signal.aborted) return;
      const decision = documentRefreshDecision({
        currentRevision: documentState.current.revision,
        nextRevision: result.revision,
        currentText: documentState.current.text,
        nextText: result.text,
        reading: readingRef.current,
        now: performance.now(),
        autoApplyUntil: autoApplyUntil.current,
      });
      if (decision === "ignore") setPendingUpdate(undefined);
      else if (decision === "apply") applyInPlace.current(result);
      else setPendingUpdate({ kind: "content", result });
    }).catch((reason) => {
      if (controller.signal.aborted) return;
      if (reason instanceof DOMException && reason.name === "AbortError") {
        return;
      }
      const status = reason instanceof CodeApiError ? reason.status : undefined;
      if (shouldCloseUnavailableSource(target.kind, status)) {
        if (unseen()) {
          mutateMobileReview(sessionId, "close", { path: target.path });
        } else {
          setPendingUpdate({ kind: "removed" });
        }
        return;
      }
      // Keep the current snapshot on screen. A snapshot race or reconnect
      // retries briefly; the next worktree change revalidates again.
      if (
        isTransientCodeApiStatus(status) && refreshRetries.current.count < 2
      ) {
        refreshRetries.current.count += 1;
        const delay = refreshRetries.current.count === 1 ? 400 : 1_200;
        globalThis.setTimeout(() => {
          if (!controller.signal.aborted) {
            setRefreshRetry((value) => value + 1);
          }
        }, delay);
      }
    });
    return () => controller.abort();
  }, [dataRevision, refreshRetry]);

  useEffect(() => {
    const wasReading = readingRef.current;
    readingRef.current = reading;
    if (reading) {
      // Returning to Review revalidates at once; that change was never seen.
      if (!wasReading) {
        autoApplyUntil.current = performance.now() +
          DOCUMENT_REFRESH_RESUME_GRACE_MS;
      }
      return;
    }
    const pending = pendingRef.current;
    if (!pending) return;
    if (pending.kind === "removed") {
      mutateMobileReview(sessionId, "close", { path: target.path });
    } else {
      applyInPlace.current(pending.result);
    }
  }, [reading]);

  // A remote Machine or its adapter can reconnect after the bounded initial
  // retries. Recover the preserved tab when the app becomes usable again so a
  // transient outage never leaves Code Review stuck behind a manual button.
  useEffect(() => {
    if (!error) return undefined;
    const retry = (): void => {
      if (document.visibilityState !== "visible" || !navigator.onLine) return;
      fileRetry.current.count = 0;
      setError(undefined);
      setReloadKey((value) => value + 1);
    };
    globalThis.addEventListener("online", retry);
    globalThis.addEventListener("pageshow", retry);
    document.addEventListener("visibilitychange", retry);
    return () => {
      globalThis.removeEventListener("online", retry);
      globalThis.removeEventListener("pageshow", retry);
      document.removeEventListener("visibilitychange", retry);
    };
  }, [error]);

  const loadMore = (): void => {
    if (!nextCursor || loadingMore) return;
    const controller = new AbortController();
    pageController.current?.abort();
    pageController.current = controller;
    setLoadingMore(true);
    setLoadMoreError(false);
    const request = target.kind === "diff"
      ? fetchCodeDiffPage(sessionId, nextCursor, controller.signal)
      : fetchCodeFilePage(
        sessionId,
        target.path,
        nextCursor,
        controller.signal,
      );
    void request
      .then((page) => {
        if (controller.signal.aborted) return;
        if (page.revision !== revision) {
          throw new Error("Document revision changed");
        }
        setText((current) => current + page.text);
        setNextCursor(page.nextCursor);
        setLimited(page.limited ?? false);
        setTruncated(page.truncated);
        if (target.kind === "diff" && !page.nextCursor) {
          onRevision(page.revision);
        }
      })
      .catch((reason) => {
        if (controller.signal.aborted) return;
        if (reason instanceof DOMException && reason.name === "AbortError") {
          return;
        }
        if (
          reason instanceof CodeApiError &&
          (reason.status === 409 || reason.status === 410)
        ) {
          setReloadKey((value) => value + 1);
          return;
        }
        setLoadMoreError(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoadingMore(false);
      });
  };

  if (loading) {
    return (
      <Box sx={{ display: "grid", placeItems: "center", flex: 1 }}>
        <CircularProgress size={24} />
      </Box>
    );
  }
  if (error) {
    const missing = error === 404;
    const unsupported = error === 415;
    const invalid = error === 400;
    const retryable = !missing && !unsupported && !invalid;
    return (
      <Box
        role="alert"
        sx={{
          flex: 1,
          minHeight: 0,
          display: "grid",
          placeItems: "center",
          px: 3,
          py: 6,
        }}
      >
        <Stack
          alignItems="center"
          spacing={1.25}
          sx={{ width: "min(100%, 420px)", textAlign: "center" }}
        >
          <Box
            sx={{
              width: 48,
              height: 48,
              display: "grid",
              placeItems: "center",
              borderRadius: "50%",
              color: "error.main",
              bgcolor: (theme) => alpha(theme.palette.error.main, 0.1),
            }}
          >
            <Refresh />
          </Box>
          <Typography variant="subtitle1" fontWeight={750}>
            {missing
              ? "This file is no longer available"
              : unsupported
              ? "This is a binary file"
              : invalid
              ? "This file can’t be opened"
              : target.kind === "diff"
              ? "Couldn’t load this diff"
              : "Couldn’t load this file"}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {missing
              ? "It may have been moved or deleted in the worktree."
              : unsupported
              ? "Cowboy can preview text, images, SVG, and Mermaid files, but not compiled programs or other binary data."
              : invalid
              ? "The saved path or file snapshot is no longer valid."
              : "The connection may have been interrupted. Your tabs and reading position are preserved."}
          </Typography>
          {retryable && (
            <Button
              variant="contained"
              startIcon={<Refresh />}
              onClick={() => {
                fileRetry.current.count = 0;
                setError(undefined);
                setReloadKey((value) => value + 1);
              }}
              sx={{ mt: 0.75, textTransform: "none", borderRadius: 2 }}
            >
              Try again
            </Button>
          )}
        </Stack>
      </Box>
    );
  }
  const candidateChip = (
    candidate: CodeInspectCandidate,
  ): React.JSX.Element => {
    const selected = candidate.row === inspectTarget?.row &&
      candidate.column === inspectTarget.column;
    return (
      <Chip
        key={`${candidate.row}:${candidate.column}:${candidate.label}`}
        label={candidate.label}
        size="small"
        color={selected ? "primary" : "default"}
        variant={selected ? "filled" : "outlined"}
        clickable={!selected}
        onPointerDown={(event) => event.stopPropagation()}
        onClick={selected ? undefined : () => {
          navigationHaptic();
          inspectPoint(candidate, inspectCandidates, false);
        }}
        sx={{
          flex: "0 0 auto",
          height: 32,
          borderRadius: 1.75,
          fontFamily: "var(--cowboy-font-mono)",
          fontWeight: selected ? 700 : 500,
        }}
      />
    );
  };
  const candidateSwitcher = inspectCandidates.length > 1
    ? (
      <Stack
        component="span"
        data-symbol-candidate-switcher
        direction="row"
        useFlexGap
        gap={0.5}
        onPointerDown={(event) => event.stopPropagation()}
        sx={{
          flex: "1 1 0",
          minWidth: 0,
          maxWidth: "100%",
          overflowX: "auto",
          // An overflow-x scrollport clips exactly at its content box. Leave
          // a small block-axis gutter so high-DPI WebKit does not shave the
          // lower subpixel of outlined chip borders while horizontally
          // scrolling long symbol names.
          py: "2px",
          overscrollBehaviorX: "contain",
          touchAction: "pan-x",
          WebkitOverflowScrolling: "touch",
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        {inspectCandidates.map(candidateChip)}
      </Stack>
    )
    : null;
  const selectedCandidate =
    inspectCandidates.find((candidate) =>
      candidate.row === inspectTarget?.row &&
      candidate.column === inspectTarget.column
    ) ?? inspectCandidates[0];
  const alternativeCandidates = selectedCandidate
    ? inspectCandidates.filter((candidate) => candidate !== selectedCandidate)
    : inspectCandidates;
  const mobileCandidateSelector = inspectCandidates.length > 1
    ? (
      <Box
        data-mobile-symbol-candidates
        sx={{
          mb: 1.5,
          pb: 1.5,
          borderBottom: 1,
          borderColor: "divider",
        }}
      >
        <ButtonBase
          data-mobile-symbol-current
          aria-expanded={inspectCandidatesExpanded}
          aria-label={inspectCandidatesExpanded
            ? "Collapse symbol list"
            : `Show ${alternativeCandidates.length} other symbols`}
          onClick={() => {
            navigationHaptic();
            setInspectCandidatesExpanded((expanded) => !expanded);
          }}
          sx={(theme) => ({
            width: "100%",
            minHeight: 48,
            px: 1.25,
            py: 0.75,
            gap: 1,
            justifyContent: "flex-start",
            borderRadius: 1.75,
            textAlign: "left",
            bgcolor: alpha(theme.palette.primary.main, 0.08),
          })}
        >
          <Box
            aria-hidden="true"
            sx={{
              width: 8,
              height: 8,
              flex: "0 0 auto",
              borderRadius: "50%",
              bgcolor: "primary.main",
            }}
          />
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ display: "block", lineHeight: 1.1 }}
            >
              Current symbol
            </Typography>
            <HintTooltip title={selectedCandidate?.label}>
              <Typography
                noWrap
                sx={{
                  mt: 0.25,
                  color: "primary.main",
                  fontFamily: "var(--cowboy-font-mono)",
                  fontSize: "0.875rem",
                  fontWeight: 700,
                }}
              >
                {selectedCandidate?.label}
              </Typography>
            </HintTooltip>
          </Box>
          <Typography
            variant="caption"
            color="primary.main"
            sx={{ flex: "0 0 auto", fontWeight: 700 }}
          >
            +{alternativeCandidates.length}
          </Typography>
          {inspectCandidatesExpanded
            ? <KeyboardArrowUp sx={{ fontSize: "1.25rem" }} />
            : <KeyboardArrowDown sx={{ fontSize: "1.25rem" }} />}
        </ButtonBase>
        {inspectCandidatesExpanded && (
          <Box
            data-mobile-symbol-grid
            sx={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(7rem, 1fr))",
              gap: 0.75,
              mt: 1,
              maxHeight: "min(32vh, 224px)",
              overflowY: "auto",
              overscrollBehaviorY: "contain",
              touchAction: "pan-y",
              WebkitOverflowScrolling: "touch",
            }}
          >
            {inspectCandidates.map((candidate) => {
              const selected = candidate === selectedCandidate;
              return (
                <HintTooltip
                  key={`${candidate.row}:${candidate.column}:${candidate.label}`}
                  title={candidate.label}
                >
                  <ButtonBase
                    data-mobile-symbol-choice
                    data-selected={selected ? "true" : undefined}
                    aria-pressed={selected}
                    onClick={selected ? undefined : () => {
                      navigationHaptic();
                      inspectPoint(candidate, inspectCandidates, false);
                    }}
                    sx={(theme) => ({
                      minWidth: 0,
                      minHeight: 44,
                      px: 1.25,
                      justifyContent: "flex-start",
                      border: 1,
                      borderColor: selected ? "primary.main" : "divider",
                      borderRadius: 1.5,
                      color: selected ? "primary.main" : "text.primary",
                      bgcolor: selected
                        ? alpha(theme.palette.primary.main, 0.12)
                        : "background.paper",
                    })}
                  >
                    <Typography
                      noWrap
                      sx={{
                        minWidth: 0,
                        fontFamily: "var(--cowboy-font-mono)",
                        fontSize: "0.8125rem",
                        fontWeight: selected ? 700 : 550,
                      }}
                    >
                      {candidate.label}
                    </Typography>
                  </ButtonBase>
                </HintTooltip>
              );
            })}
          </Box>
        )}
      </Box>
    )
    : null;
  const navigationButton = (
    kind: CodeNavigationKind,
    label: string,
  ): React.JSX.Element => {
    const availability = navigationAvailability[kind];
    return (
      <Button
        size="small"
        disabled={availability !== "available"}
        data-navigation-kind={kind}
        data-navigation-status={availability}
        aria-label={availability === "available"
          ? label
          : availability === "checking"
          ? `${label} checking`
          : `${label} unavailable`}
        onClick={() => navigate(kind)}
      >
        {label}
      </Button>
    );
  };
  const symbolContent = (
    <Stack spacing={1.5} sx={{ pb: 2 }}>
      {hoverLoading
        ? (
          <Stack alignItems="center" sx={{ py: 4 }}>
            <CircularProgress size={24} />
          </Stack>
        )
        : hoverError
        ? (
          <Stack spacing={1.5} alignItems="flex-start" sx={{ py: 2 }}>
            <Typography color="text.secondary">
              Symbol information is temporarily unavailable.
            </Typography>
            <Button
              size="small"
              variant="outlined"
              onClick={() => {
                if (inspectTarget) {
                  inspectPoint(inspectTarget, inspectCandidates, false);
                }
              }}
            >
              Retry
            </Button>
          </Stack>
        )
        : hover?.contents.length
        ? hover.contents.map((rawBlock, index) => {
          const block = presentHoverBlock(rawBlock);
          return block.markdown
            ? (
              <Box
                key={index}
                sx={{
                  minWidth: 0,
                  fontSize: "0.9375rem",
                  lineHeight: 1.65,
                  "& > :first-child": { mt: 0 },
                  "& > :last-child": { mb: 0 },
                  "& p": { my: 1.25 },
                  "& h1, & h2, & h3, & h4": {
                    mt: 2.5,
                    mb: 1,
                    fontSize: "1rem",
                    lineHeight: 1.35,
                    letterSpacing: "-0.01em",
                  },
                  "& img": {
                    maxHeight: 28,
                    width: "auto",
                    borderRadius: 1,
                  },
                  "& pre": {
                    border: 1,
                    borderColor: "divider",
                    borderRadius: 1,
                    boxShadow: "inset 0 1px 0 rgba(255,255,255,0.04)",
                  },
                  "& .cowboy-copy-btn": {
                    borderRadius: 1,
                  },
                  "& hr": {
                    border: 0,
                    borderTop: 1,
                    borderColor: "divider",
                    my: 1.5,
                  },
                }}
              >
                <Markdown text={block.text} touchWrap />
              </Box>
            )
            : (
              <Box
                key={index}
                component="pre"
                sx={{
                  m: 0,
                  p: 1.5,
                  overflowX: "auto",
                  borderRadius: 1,
                  bgcolor: "action.hover",
                  fontFamily: "var(--cowboy-font-mono)",
                  whiteSpace: "pre-wrap",
                }}
              >
                {block.text}
              </Box>
            );
        })
        : (
          <Typography color="text.secondary" sx={{ py: 3 }}>
            No symbol information is available at this location.
          </Typography>
        )}
      {usingOwned && hoverOpen && inspectTarget && owned.identity && (
        <ReviewNavigation
          key={`${inspectTarget.row}:${inspectTarget.column}`}
          intelligence={owned}
          point={inspectTarget}
        />
      )}
      {!usingOwned && !hoverLoading && !hoverError && (
        <Stack direction="row" useFlexGap flexWrap="wrap" gap={1}>
          {navigationButton("definition", "Definition")}
          {navigationButton("declaration", "Declaration")}
          {navigationButton("typeDefinition", "Type")}
          {navigationButton("implementation", "Implementations")}
          {navigationButton("references", "References")}
        </Stack>
      )}
      {navigation.map((location) => (
        <Button
          key={`${location.path}:${location.start.row}:${location.start.column}`}
          variant="outlined"
          sx={{ justifyContent: "flex-start", textTransform: "none" }}
          onClick={() => {
            setHoverOpen(false);
            if (inspectTarget) onNavigate(location, inspectTarget);
          }}
        >
          {location.path}:{location.start.row + 1}
        </Button>
      ))}
    </Stack>
  );
  return (
    <Stack sx={{ flex: 1, minHeight: 0, position: "relative", pl: "env(safe-area-inset-left, 0px)", pr: "env(safe-area-inset-right, 0px)" }}>
      {pendingUpdate && (
        <Box
          role="status"
          sx={{
            position: "absolute",
            left: 0,
            right: 0,
            bottom: 16,
            zIndex: 2,
            mx: "auto",
            width: "fit-content",
            maxWidth: "calc(100% - 32px)",
            display: "flex",
            alignItems: "center",
            gap: 1,
            pl: 1.75,
            pr: 0.5,
            py: 0.5,
            borderRadius: 999,
            border: 1,
            borderColor: "divider",
            bgcolor: "background.paper",
          }}
        >
          <Typography variant="body2" color="text.secondary" noWrap>
            {pendingUpdate.kind === "removed"
              ? "This file was removed"
              : "This file changed"}
          </Typography>
          <Button
            size="small"
            startIcon={pendingUpdate.kind === "content"
              ? <Refresh fontSize="small" />
              : undefined}
            onClick={() => {
              if (pendingUpdate.kind === "removed") {
                mutateMobileReview(sessionId, "close", { path: target.path });
              } else {
                applyInPlace.current(pendingUpdate.result);
              }
            }}
            sx={{ textTransform: "none", borderRadius: 999 }}
          >
            {pendingUpdate.kind === "removed" ? "Close" : "Refresh"}
          </Button>
        </Box>
      )}
      {usingOwned && !mediaPreview && !markdownPreview &&
        previewKind !== "mermaid" && (isPatch
          ? <ReviewDiffCodeStatus diff={diff} intelligence={owned} />
          : target.kind === "source" && (
            <ReviewCodeStatus intelligence={owned} />
          ))}
      {target.kind === "source" && !markdownPreview && (
        <ReviewCheckStatus sessionId={sessionId} path={target.path} />
      )}
      {target.kind === "source" && (
        <ReviewOutline
          open={outlineOpen}
          onClose={onOutlineClose}
          sessionId={sessionId}
          path={target.path}
          currentLine={outlineLine}
          documentIdentity={displayText}
          onSelect={onOutlineSelect}
          owned={bufferMode !== "legacy"
            ? {
              identity: owned.status === "navigation"
                ? undefined
                : owned.identity,
              read: owned.outline,
            }
            : undefined}
        />
      )}
      <Box
        ref={outerScrollRef}
        data-review-document-content
        data-mobile-overflow-layer={markdownPreview ||
            previewKind === "mermaid" || mediaPreview ||
            settings.softWrap
          ? "true"
          : undefined}
        sx={{
          flex: 1,
          minHeight: 0,
          overflow: markdownPreview || previewKind === "mermaid" ||
              mediaPreview || settings.softWrap
            ? "auto"
            : "hidden",
          // The drawer and pager own non-passive touchmove recognizers on
          // ancestors. Without an axis contract iOS waits for them before it
          // starts this native scroll, so a vertical flick lags the finger.
          // Same contract as Transcript and the Review lists; horizontal
          // drags still reach the recognizers.
          ...(outerScrollable && { touchAction: "pan-y pinch-zoom" }),
        }}
        onScroll={outerScrollable
          ? (event) => {
            if (appliedOuterRestore.current !== outerScrollKey) return;
            rememberScrollPosition(
              outerScrollKey,
              event.currentTarget.scrollTop,
            );
          }
          : undefined}
      >
        {mediaPreview && (previewKind === "image" || previewKind === "svg")
          ? (
            <ReviewMediaPreview
              key={`${sessionId}:${target.path}`}
              sessionId={sessionId}
              path={target.path}
              kind={previewKind}
            />
          )
          : previewKind === "mermaid"
          ? (
            <MermaidDiagram
              key={`${sessionId}:${target.path}`}
              source={displayText}
            />
          )
          : markdownPreview
          ? (
            <Box
              component="article"
              data-markdown-review-preview
              sx={markdownPreviewSx(settings.markdownSoftWrap)}
            >
              {
                /* Off by default: code blocks and tables scroll natively.
                   On fits them to the column, so the peek holds no nested
                   iOS ScrollViews (docs/mobile-spatial-presentation.md 4.2). */
              }
              <Markdown
                text={displayText}
                touchWrap={settings.markdownSoftWrap}
                frontmatter
                onLinkClick={onMarkdownLink}
              />
            </Box>
          )
          : (
            <Suspense
              fallback={
                <Box sx={{ display: "grid", placeItems: "center", height: 1 }}>
                  <CircularProgress size={24} />
                </Box>
              }
            >
              <CodeViewer
                sessionId={sessionId}
                text={displayText}
                kind={target.kind}
                path={target.path}
                softWrap={settings.softWrap}
                fontSize={settings.codeFontSize}
                revealLine={target.kind === "diff"
                  ? hunks[hunkIndex]
                  : target.revealLine}
                revealRange={target.kind === "source"
                  ? target.revealRange
                  : undefined}
                revealRequestId={target.kind === "source"
                  ? target.revealRequestId
                  : undefined}
                languageData={target.kind === "source"
                  ? usingOwned ? owned.language : languageData
                  : undefined}
                diagnostics={settings.diagnostics}
                inlayHints={settings.inlayHints}
                semanticHighlighting={settings.semanticHighlighting}
                onInspect={bufferMode && bufferMode !== "unavailable" &&
                    (target.kind === "source" ||
                      (target.kind === "diff" && (usingOwned
                        ? !!diff.projection && !!owned.identity
                        : target.scope === "unstaged")))
                  ? inspectCandidatesOrPoint
                  : undefined}
                mapDiffPoint={usingOwned ? diff.point : undefined}
                {...(onSendSelection
                  ? {
                    selection: lineSelection,
                    onToggleLine: toggleSelectedLine,
                  }
                  : {})}
                onVisibleLine={target.kind === "source"
                  ? persistVisibleLine
                  : undefined}
                scrollRestoreKey={editorScrollKey}
                savedScrollTop={readScrollPosition(editorScrollKey)}
                onScrollTopChange={rememberScrollPosition}
              />
            </Suspense>
          )}
        {lineSelection && onSendSelection && (
          <ReviewSelectionBar
            selection={lineSelection}
            onClear={() => setLineSelection(undefined)}
            onSend={() => {
              onSendSelection(selectionMessage(lineSelection));
              setLineSelection(undefined);
            }}
            onExplain={onExplainSelection
              ? async (kind) => {
                await onExplainSelection(target.path, selectionMessage(lineSelection), kind);
                setLineSelection(undefined);
              }
              : undefined}
          />
        )}
        {surface.kind === "mobile"
          ? (
            <Sheet
              open={hoverOpen}
              onClose={() => setHoverOpen(false)}
              title={inspectCandidates.length > 1
                ? `Symbols · ${inspectCandidates.length}`
                : "Symbol"}
              forceSheet
              mobileDismiss="footer"
              portal
            >
              {mobileCandidateSelector}
              {symbolContent}
            </Sheet>
          )
          : (
            <Popover
              open={hoverOpen}
              onClose={() => setHoverOpen(false)}
              anchorReference="anchorPosition"
              anchorPosition={inspectAnchor ?? { top: 0, left: 0 }}
              transformOrigin={{ vertical: "top", horizontal: "left" }}
              slotProps={{
                paper: {
                  sx: {
                    width: "min(680px, calc(100vw - 40px))",
                    maxHeight: "min(78vh, 760px)",
                    p: 0,
                    overflow: "hidden",
                    borderRadius: 2,
                    bgcolor: "background.paper",
                    backgroundImage: "none",
                    border: 1,
                    borderColor: "divider",
                    boxShadow: 12,
                  },
                },
              }}
            >
              <Box
                sx={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  gap: 2,
                  px: 2.5,
                  py: 1.75,
                  borderBottom: 1,
                  borderColor: "divider",
                  bgcolor: "background.paper",
                }}
              >
                <Box sx={{ minWidth: 0 }}>
                  <Typography variant="subtitle1" fontWeight={750}>
                    Symbol
                  </Typography>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{
                      display: "block",
                      mt: 0.25,
                      fontFamily: "var(--cowboy-font-mono)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {target.path}
                  </Typography>
                </Box>
                {candidateSwitcher}
              </Box>
              <Box
                sx={{
                  px: 2.5,
                  pt: 2,
                  overflowY: "auto",
                  maxHeight: "calc(min(78vh, 760px) - 74px)",
                }}
              >
                {symbolContent}
              </Box>
            </Popover>
          )}
      </Box>
      {(truncated || counts || nextCursor || loadMoreError) && (
        <ReviewDocumentControls
          counts={counts} hunkCount={hunks.length} hunkIndex={hunkIndex}
          onPrevious={() => setHunkIndex((value) => value - 1)}
          onNext={() => setHunkIndex((value) => value + 1)}
          hasMore={!!nextCursor} loadingMore={loadingMore} loadMoreError={loadMoreError}
          onLoadMore={loadMore} limited={limited} diff={target.kind === "diff"}
        />
      )}
    </Stack>
  );
}
