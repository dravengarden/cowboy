import {
  ArrowDownward,
  ArrowUpward,
  ChevronLeft,
  ChevronRight,
  Close,
  EditOutlined,
  ListAltOutlined,
  Search,
} from "@mui/icons-material";
import {
  alpha,
  Box,
  Button,
  CircularProgress,
  IconButton,
  InputAdornment,
  List,
  ListItemButton,
  ListItemText,
  Paper,
  Skeleton,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  DetentSheet,
  MobileSheetActionGroup,
} from "../_shell";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "../desktop/commands/DesktopCommandProvider";
import { DesktopModal } from "../desktop/DesktopModal";
import { desktopEmbeddedControlSx } from "../desktop/DesktopEmbeddedControl";
import { useOptionalDesktopWorkspace } from "../desktop/DesktopWorkspaceController";
import { derive } from "../derive";
import { Kbd } from "../Kbd";
import type { Envelope, Status } from "../protocol";
import {
  isQuestionPageLoaded,
  loadQuestionPage,
  loadPreviousQuestionPage,
  useStoreSelector,
} from "../store";
import { setSticky } from "../stickyStore";
import { Transcript } from "../Transcript";
import { useReliableTouchTap } from "../useReliableTouchTap";
import {
  beginExplorePageLoading,
  setExplorePage,
  setExploreAtTail,
  navigateExplorePage,
  resolveExplorePageStart,
  resolveProjectionAnchor,
  resolveExploreFollowUp,
  useExploreSessionState,
} from "./exploreStore";
import {
  authoritativeTailPageId,
  completePageBeforeItem,
  deriveQuestionPages,
  groupQuestionPages,
  indexedQuestionPagePosition,
  mergeQuestionPageDirectory,
  pageContainingItemKey,
  presentQuestionPageDirectory,
  type QuestionPage,
} from "./questionPages";
import {
  nextFollowedTailPage,
  pageStartHandshakeIdentity,
  questionPageNeedsRestore,
  shouldAdoptLoadedPage,
} from "./retainedPage";

const EMPTY_TIMELINE: Envelope[] = [];

function PageTurnFooter({
  currentOrdinal,
  total,
  previousDisabled,
  nextDisabled,
  previousQuestion,
  nextQuestion,
  loadingPrevious,
  loadingNext,
  onPrevious,
  onNext,
  desktop,
}: {
  currentOrdinal: number;
  total: number;
  previousDisabled: boolean;
  nextDisabled: boolean;
  previousQuestion: string | null;
  nextQuestion: string | null;
  loadingPrevious: boolean;
  loadingNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
  desktop: boolean;
}): React.JSX.Element | null {
  if (previousDisabled && nextDisabled) return null;
  const actionSx = desktop
    ? {
      ...desktopEmbeddedControlSx(),
      minHeight: 52,
      px: 1.25,
      textTransform: "none",
    }
    : {
      minHeight: 64,
      px: 1.25,
      border: 1,
      borderColor: "divider",
      borderRadius: 1,
      bgcolor: "transparent",
      textTransform: "none",
      "&:active": { bgcolor: "action.selected" },
    };
  const directionLabel = (
    direction: "previous" | "next",
    question: string | null,
    loading: boolean,
  ): React.JSX.Element => {
    const previous = direction === "previous";
    return (
      <Stack
        direction="row"
        spacing={0.75}
        alignItems="center"
        sx={{ minWidth: 0, width: "100%", position: "relative" }}
      >
        {previous && (loading
          ? <CircularProgress size={16} sx={{ flexShrink: 0 }} />
          : <ChevronLeft sx={{ flexShrink: 0 }} />)}
        <Box
          sx={{
            minWidth: 0,
            flex: 1,
            textAlign: previous ? "left" : "right",
            ...(desktop && (previous ? { pr: 4 } : { pl: 4 })),
          }}
        >
          <Typography
            component="span"
            variant="caption"
            sx={{ display: "block", fontWeight: 700, lineHeight: 1.2 }}
          >
            {previous ? "Previous" : "Next"}
          </Typography>
          <Typography
            component="span"
            variant="caption"
            color="text.secondary"
            title={question ?? "Untitled question"}
            sx={{
              display: "-webkit-box",
              overflow: "hidden",
              overflowWrap: "anywhere",
              WebkitBoxOrient: "vertical",
              WebkitLineClamp: desktop ? 1 : 2,
              lineHeight: 1.25,
            }}
          >
            {question ?? "Untitled question"}
          </Typography>
        </Box>
        {desktop && (
          <Box
            sx={{
              position: "absolute",
              top: 0,
              ...(previous ? { right: 0 } : { left: 0 }),
              display: "inline-flex",
              alignItems: "center",
              pointerEvents: "none",
              "& kbd": { ml: "0 !important" },
            }}
          >
            <Kbd keys={previous ? "[" : "]"} variant="global" />
          </Box>
        )}
        {!previous && (loading
          ? <CircularProgress size={16} sx={{ flexShrink: 0 }} />
          : <ChevronRight sx={{ flexShrink: 0 }} />)}
      </Stack>
    );
  };
  return (
    <Box
      component="nav"
      aria-label="Page footer navigation"
      data-page-turn-footer
      sx={{
        mt: 2.5,
        mb: 0.5,
        pt: 1.5,
        borderTop: 1,
        borderColor: "divider",
        display: "grid",
        gridTemplateColumns: "minmax(0, 1fr) auto minmax(0, 1fr)",
        alignItems: "center",
        gap: 1,
      }}
    >
      {previousDisabled
        ? <Box aria-hidden />
        : (
          <Button
            aria-label={`Previous question: ${previousQuestion ?? "Untitled question"}`}
            disabled={loadingPrevious || loadingNext}
            onClick={onPrevious}
            sx={{ ...actionSx, justifySelf: "stretch", minWidth: 0, width: "100%" }}
          >
            {directionLabel("previous", previousQuestion, loadingPrevious)}
          </Button>
        )}
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ fontWeight: 700, fontVariantNumeric: "tabular-nums" }}
      >
        {currentOrdinal} / {total}
      </Typography>
      {nextDisabled
        ? <Box aria-hidden />
        : (
          <Button
            aria-label={`Next question: ${nextQuestion ?? "Untitled question"}`}
            disabled={loadingPrevious || loadingNext}
            onClick={onNext}
            sx={{ ...actionSx, justifySelf: "stretch", minWidth: 0, width: "100%" }}
          >
            {directionLabel("next", nextQuestion, loadingNext)}
          </Button>
        )}
    </Box>
  );
}

interface QuestionPageSummary {
  id: string;
  title: string;
  ordinal: number;
}

function DesktopQuestionDirectoryCommand({
  available,
  toggle,
}: {
  available: boolean;
  toggle: () => void;
}): null {
  const command = useMemo<DesktopCommand>(() => ({
    id: "conversation.toggleQuestionDirectory",
    title: "Toggle Question Navigator",
    description: "Find and jump to a question page",
    group: "Conversation",
    shortcut: "P",
    contexts: ["conversation"],
    when: () => available,
    disabledReason: "No question pages are available",
    run: toggle,
  }), [available, toggle]);
  useDesktopCommand(command);
  return null;
}

function useQuestionPageIndex(
  sessionId: string,
  revisionKey: string | undefined,
): {
  data: {
    total: number;
    exact: boolean;
    pages: QuestionPageSummary[];
    nextBeforeSeq: number | null;
  } | null;
  loading: boolean;
  loadingEarlier: boolean;
  loadEarlier: () => Promise<void>;
} {
  const [state, setState] = useState<{
    data: {
      total: number;
      exact: boolean;
      pages: QuestionPageSummary[];
      nextBeforeSeq: number | null;
    } | null;
    loading: boolean;
    loadingEarlier: boolean;
    sessionId: string;
  }>({ data: null, loading: true, loadingEarlier: false, sessionId });
  const data = state.sessionId === sessionId ? state.data : null;
  useEffect(() => {
    const controller = new AbortController();
    setState((current) => ({
      data: current.sessionId === sessionId ? current.data : null,
      loading: true,
      loadingEarlier: false,
      sessionId,
    }));
    void fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/question-pages`,
      { signal: controller.signal },
    )
      .then((response) => {
        if (!response.ok) {
          throw new Error(`question pages: ${String(response.status)}`);
        }
        return response.json() as Promise<{
          total: number;
          exact: boolean;
          pages: Array<{ id: number; title: string; ordinal: number }>;
          next_before_seq: number | null;
        }>;
      })
      .then((next) =>
        setState({
          data: {
            total: next.total,
            exact: next.exact,
            pages: next.pages.map((page) => ({ ...page, id: String(page.id) })),
            nextBeforeSeq: next.next_before_seq,
          },
          loading: false,
          loadingEarlier: false,
          sessionId,
        })
      )
      .catch((error: unknown) => {
        if (!(error instanceof DOMException && error.name === "AbortError")) {
          console.warn("Failed to load exact question page count", error);
          setState((current) => ({
            ...current,
            loading: false,
          }));
        }
      });
    return () => controller.abort();
  }, [sessionId, revisionKey]);
  const loadEarlier = useCallback(async (): Promise<void> => {
    const cursor = data?.nextBeforeSeq;
    if (cursor === null || cursor === undefined || state.loadingEarlier) return;
    setState((current) => ({ ...current, loadingEarlier: true }));
    try {
      const response = await fetch(
        `/api/sessions/${encodeURIComponent(sessionId)}/question-pages?limit=64&before=${String(cursor)}`,
      );
      if (!response.ok) throw new Error(`question pages: ${String(response.status)}`);
      const next = (await response.json()) as {
        total: number;
        exact: boolean;
        pages: Array<{ id: number; title: string; ordinal: number }>;
        next_before_seq: number | null;
      };
      setState((current) => {
        if (current.sessionId !== sessionId || !current.data) return current;
        const existing = new Set(current.data.pages.map((page) => page.id));
        const earlier = next.pages
          .map((page) => ({ ...page, id: String(page.id) }))
          .filter((page) => !existing.has(page.id));
        return {
          ...current,
          loadingEarlier: false,
          data: {
            total: next.total,
            exact: next.exact,
            pages: [...earlier, ...current.data.pages],
            nextBeforeSeq: next.next_before_seq,
          },
        };
      });
    } catch (error) {
      console.warn("Failed to load earlier question titles", error);
      setState((current) => ({ ...current, loadingEarlier: false }));
    }
  }, [data?.nextBeforeSeq, sessionId, state.loadingEarlier]);
  return {
    data,
    loading: state.loading || state.sessionId !== sessionId,
    loadingEarlier: state.loadingEarlier && state.sessionId === sessionId,
    loadEarlier,
  };
}

export interface ExploreTranscriptProps {
  sessionId: string;
  timeline: Envelope[];
  status: Status;
  provider: string;
  cwd: string;
  loading: boolean;
  connected: boolean;
  topInset?: string | undefined;
  bottomInset?: string | undefined;
  onScrollableChange?: ((scrollable: boolean) => void) | undefined;
  desktop: boolean;
}

function usePages(
  sessionId: string,
  timeline: Envelope[],
): {
  pages: QuestionPage[];
  current: QuestionPage | null;
  currentIndex: number;
  select: (id: string) => void;
  navigate: (id: string) => void;
} {
  const {
    pageId,
    pageParents,
    pendingFollowUp,
    transitionAnchorKey,
  } = useExploreSessionState(sessionId);
  const basePages = useMemo(() => {
    const derived = deriveQuestionPages(derive(timeline));
    const rooted = derived.filter((page) => page.questionCount > 0);
    // A bounded history window may begin with lifecycle events whose question
    // root is older. Once a real root is present, that provisional projection
    // must not become a separately navigable Page View page.
    return rooted.length > 0 ? rooted : derived;
  }, [timeline]);
  useEffect(() => {
    if (pendingFollowUp) {
      resolveExploreFollowUp(sessionId, basePages.map((page) => page.id));
    }
  }, [basePages, pendingFollowUp, sessionId]);
  const pages = useMemo(
    () => groupQuestionPages(basePages, pageParents),
    [basePages, pageParents],
  );
  const tailPageId = pages.at(-1)?.id ?? null;
  const previousTailPageIdRef = useRef<string | null>(tailPageId);
  useEffect(() => {
    const previousTailPageId = previousTailPageIdRef.current;
    previousTailPageIdRef.current = tailPageId;
    const nextPageId = nextFollowedTailPage(
      pageId,
      previousTailPageId,
      tailPageId,
      pendingFollowUp !== null,
    );
    if (nextPageId !== null) setExplorePage(sessionId, nextPageId);
  }, [pageId, pendingFollowUp, sessionId, tailPageId]);
  const transitionPage = pageContainingItemKey(pages, transitionAnchorKey);
  const selectedPageId = transitionPage?.id ?? pageId;
  const selectedIndex = selectedPageId
    ? pages.findIndex((page) => page.id === selectedPageId)
    : -1;
  const currentIndex = selectedIndex >= 0
    ? selectedIndex
    : Math.max(0, pages.length - 1);
  const current = pages[currentIndex] ?? null;

  useEffect(() => {
    if (transitionAnchorKey && current) {
      resolveProjectionAnchor(sessionId, current.id);
      return;
    }
    const currentId = current?.id ?? null;
    // A retained page can be outside the bounded live timeline while its title
    // is already known. Do not overwrite that durable selection with the newest
    // loaded page during the short lazy-load window.
    if (
      currentId !== null &&
      shouldAdoptLoadedPage(
        pageId,
        currentId,
        pages.map((page) => page.id),
      )
    ) {
      setExplorePage(sessionId, currentId);
    }
  }, [current?.id, pageId, pages, sessionId, transitionAnchorKey]);

  return {
    pages,
    current,
    currentIndex,
    select: (id: string): void => {
      setSticky(sessionId, false);
      navigateExplorePage(sessionId, id);
    },
    navigate: (id: string): void => {
      setSticky(sessionId, false);
      navigateExplorePage(sessionId, id);
    },
  };
}

function PageList({
  pages,
  currentId,
  onSelect,
  dense = false,
  firstOrdinal = 1,
  hasEarlier = false,
  loadingEarlier = false,
  loadingPageId,
  onReachStart,
  onDismiss,
  onVimDismiss,
  active = true,
  searchable = true,
  descending = false,
  vimNavigation = false,
  listElementRef,
  onAwayFromBottomChange,
}: {
  pages: Array<{ id: string; title: string; ordinal?: number }>;
  currentId: string | null;
  onSelect: (id: string) => void;
  dense?: boolean;
  firstOrdinal?: number;
  hasEarlier?: boolean;
  loadingEarlier?: boolean;
  loadingPageId?: string | null;
  onReachStart?: (() => void) | undefined;
  onDismiss?: (() => void) | undefined;
  onVimDismiss?: (() => void) | undefined;
  active?: boolean;
  searchable?: boolean;
  descending?: boolean;
  vimNavigation?: boolean;
  listElementRef?: React.RefObject<HTMLUListElement | null>;
  onAwayFromBottomChange?: ((away: boolean) => void) | undefined;
}): React.JSX.Element {
  const [query, setQuery] = useState("");
  const [cursorId, setCursorId] = useState<string | null>(currentId);
  const [showEarlierLoading, setShowEarlierLoading] = useState(false);
  const selectedRef = useRef<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLUListElement | null>(null);
  const startSentinelRef = useRef<HTMLDivElement | null>(null);
  const positionedForActivationRef = useRef(false);
  const earlierRequestArmedRef = useRef(true);
  const earlierLoadingShownAtRef = useRef(0);
  const earlierLoadingHideTimerRef = useRef<number | null>(null);
  const prependAnchorRef = useRef<{
    element: HTMLElement | null;
    top: number;
    scrollHeight: number;
    scrollTop: number;
    pageCount: number;
  } | null>(null);
  const vimChordRef = useRef<number | null>(null);
  const ordinalById = useMemo(
    () =>
      new Map(
        pages.map((page, index) => [
          page.id,
          page.ordinal ?? firstOrdinal + index,
        ]),
      ),
    [firstOrdinal, pages],
  );
  useEffect(() => {
    if (!active || !vimNavigation || searchable) return undefined;
    const frame = requestAnimationFrame(() =>
      listRef.current?.focus({ preventScroll: true }));
    return () => cancelAnimationFrame(frame);
  }, [active, searchable, vimNavigation]);
  const ordered = useMemo(
    () => presentQuestionPageDirectory(pages, descending),
    [descending, pages],
  );
  const filtered = searchable && query.trim()
    ? ordered.filter((page) =>
      page.title.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())
    )
    : ordered;

  useEffect(() => {
    if (active) setCursorId(currentId);
  }, [active, currentId]);

  useEffect(() => {
    return () => {
      if (vimChordRef.current !== null) {
        globalThis.clearTimeout(vimChordRef.current);
      }
    };
  }, []);

  const updateBottomAffordance = useCallback((): void => {
    const list = listRef.current;
    if (!list || !onDismiss) {
      onAwayFromBottomChange?.(false);
      return;
    }
    onAwayFromBottomChange?.(
      list.scrollHeight - list.scrollTop - list.clientHeight > 48,
    );
    // One upward approach loads one batch. Once prepending has moved the
    // viewport clear of the boundary, arm the next deliberate approach.
    const distanceFromOlderBoundary = descending
      ? list.scrollHeight - list.scrollTop - list.clientHeight
      : list.scrollTop;
    if (distanceFromOlderBoundary > 120) earlierRequestArmedRef.current = true;
  }, [descending, onAwayFromBottomChange, onDismiss]);

  useEffect(() => {
    if (!active) {
      positionedForActivationRef.current = false;
      return undefined;
    }
    // A mobile directory should position its selected row once when the sheet
    // opens. Older-page fetches can regroup the projection and change the
    // selected page id; following every such change yanks a user who is
    // browsing backwards straight back to the newest row.
    if (!onDismiss || !positionedForActivationRef.current) {
      const list = listRef.current;
      const selected = selectedRef.current;
      if (list && selected) {
        // Never use scrollIntoView inside the transformed DetentSheet: WebKit
        // may scroll its outer body as well as this nested list. Adjust only
        // the list's own scroll position, by the minimum amount needed.
        const rowTop = selected.offsetTop;
        const rowBottom = rowTop + selected.offsetHeight;
        if (rowTop < list.scrollTop) list.scrollTop = rowTop;
        else if (rowBottom > list.scrollTop + list.clientHeight) {
          list.scrollTop = rowBottom - list.clientHeight;
        }
        const close = list
          .closest<HTMLElement>('[role="dialog"][aria-label="Question pages"]')
          ?.querySelector<HTMLElement>('button[aria-label="Close"]');
        if (close) {
          const overlap = selected.getBoundingClientRect().bottom -
            (close.getBoundingClientRect().top - 12);
          if (overlap > 0) list.scrollTop += overlap;
        }
      }
      if (onDismiss && selected) {
        positionedForActivationRef.current = true;
      }
    }
    const frame = window.requestAnimationFrame(updateBottomAffordance);
    return () => window.cancelAnimationFrame(frame);
  }, [active, currentId, filtered.length, onDismiss, updateBottomAffordance]);

  useLayoutEffect(() => {
    const frame = window.requestAnimationFrame(updateBottomAffordance);
    return () => window.cancelAnimationFrame(frame);
  }, [filtered.length, loadingEarlier, updateBottomAffordance]);

  const requestEarlier = useCallback((): void => {
    if (!onReachStart || !earlierRequestArmedRef.current) return;
    earlierRequestArmedRef.current = false;
    if (!showEarlierLoading) {
      earlierLoadingShownAtRef.current = performance.now();
      setShowEarlierLoading(true);
    }
    const list = listRef.current;
    if (list && !descending) {
      const listTop = list.getBoundingClientRect().top;
      const element = Array.from(
        list.querySelectorAll<HTMLElement>("[data-page-id]"),
      ).find((row) => row.getBoundingClientRect().bottom > listTop + 1) ?? null;
      prependAnchorRef.current = {
        element,
        top: element?.getBoundingClientRect().top ?? listTop,
        scrollHeight: list.scrollHeight,
        scrollTop: list.scrollTop,
        pageCount: pages.length,
      };
    }
    onReachStart();
  }, [descending, onReachStart, pages.length, showEarlierLoading]);

  useEffect(() => {
    if (loadingEarlier) {
      if (!showEarlierLoading) {
        earlierLoadingShownAtRef.current = performance.now();
        setShowEarlierLoading(true);
      }
      return undefined;
    }
    if (!showEarlierLoading) return undefined;
    const remaining = Math.max(
      0,
      420 - (performance.now() - earlierLoadingShownAtRef.current),
    );
    earlierLoadingHideTimerRef.current = globalThis.setTimeout(() => {
      setShowEarlierLoading(false);
      earlierLoadingHideTimerRef.current = null;
    }, remaining);
    return () => {
      if (earlierLoadingHideTimerRef.current !== null) {
        globalThis.clearTimeout(earlierLoadingHideTimerRef.current);
        earlierLoadingHideTimerRef.current = null;
      }
    };
  }, [loadingEarlier, showEarlierLoading]);

  useLayoutEffect(() => {
    const anchor = prependAnchorRef.current;
    const list = listRef.current;
    if (descending) {
      prependAnchorRef.current = null;
      return;
    }
    if (!anchor || !list || pages.length <= anchor.pageCount) return;
    // Keep the first visible row under the user's finger. WebKit otherwise
    // performs its own scroll anchoring while this code also compensates for
    // the prepended height, which applies the same offset twice and creates a
    // large empty gap. If regrouping replaced the row, fall back to the exact
    // inserted-height delta with native anchoring disabled below.
    if (anchor.element?.isConnected && list.contains(anchor.element)) {
      list.scrollTop += anchor.element.getBoundingClientRect().top - anchor.top;
    } else {
      list.scrollTop = anchor.scrollTop +
        Math.max(0, list.scrollHeight - anchor.scrollHeight);
    }
    // A successful request added a visible directory page, so the next approach
    // is eligible immediately. IntersectionObserver still gates the request by
    // the 72px boundary: while a tall viewport remains unfilled it keeps
    // backfilling; once the prepend moves the sentinel clear it stops until the
    // user deliberately scrolls back to the top.
    earlierRequestArmedRef.current = true;
    prependAnchorRef.current = null;
  }, [descending, pages.length]);

  useEffect(() => {
    const anchor = prependAnchorRef.current;
    if (!loadingEarlier && anchor && pages.length <= anchor.pageCount) {
      prependAnchorRef.current = null;
    }
  }, [loadingEarlier, pages.length]);

  useEffect(() => {
    if (query.trim() || !hasEarlier || loadingEarlier) return undefined;
    const list = listRef.current;
    const sentinel = startSentinelRef.current;
    if (!list || !sentinel) return undefined;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) requestEarlier();
      },
      {
        root: list,
        // Chronological mobile lists load older rows at the top. Desktop's
        // newest-first navigator reverses that visual order, so its older-page
        // boundary is the bottom edge instead.
        rootMargin: descending ? "0px 0px 72px" : "72px 0px 0px",
        threshold: 0,
      },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [descending, hasEarlier, loadingEarlier, query, requestEarlier]);

  const moveCursor = useCallback((index: number): void => {
    const page = filtered[Math.max(0, Math.min(filtered.length - 1, index))];
    if (!page) return;
    setCursorId(page.id);
    requestAnimationFrame(() => {
      listRef.current
        ?.querySelector<HTMLElement>(`[data-page-id="${CSS.escape(page.id)}"]`)
        ?.scrollIntoView({ block: "nearest" });
    });
  }, [filtered]);

  const onVimKeyDown = (event: React.KeyboardEvent): void => {
    if (!vimNavigation) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (
      target?.matches("input, textarea, [contenteditable='true']") ||
      target?.closest("[contenteditable='true']")
    ) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        listRef.current?.focus({ preventScroll: true });
      }
      return;
    }
    if (event.metaKey || event.altKey) return;
    const key = event.code === "KeyJ"
      ? "j"
      : event.code === "KeyK"
      ? "k"
      : event.code === "KeyH"
      ? "h"
      : event.code === "KeyL"
      ? "l"
      : event.code === "KeyD"
      ? "d"
      : event.code === "KeyU"
      ? "u"
      : event.code === "KeyF"
      ? "f"
      : event.code === "KeyB"
      ? "b"
      : event.code === "KeyG"
      ? (event.shiftKey ? "G" : "g")
      : event.code === "Slash"
      ? "/"
      : event.key;
    const cursorIndex = Math.max(
      0,
      filtered.findIndex((page) => page.id === cursorId),
    );
    if (event.ctrlKey) {
      const list = listRef.current;
      if (!list) return;
      const direction = key.toLowerCase() === "d" || key.toLowerCase() === "f"
        ? 1
        : key.toLowerCase() === "u" || key.toLowerCase() === "b"
        ? -1
        : 0;
      if (direction === 0) return;
      event.preventDefault();
      event.stopPropagation();
      const firstRow = list.querySelector<HTMLElement>("[data-page-id]");
      const rowHeight = firstRow?.getBoundingClientRect().height ?? 52;
      const visibleRows = Math.max(1, Math.floor(list.clientHeight / rowHeight));
      const rowDistance =
        key.toLowerCase() === "d" || key.toLowerCase() === "u"
          ? Math.max(1, Math.floor(visibleRows / 2))
          : Math.max(1, visibleRows - 1);
      moveCursor(cursorIndex + direction * rowDistance);
      return;
    }
    if (key === "j" || key === "k") {
      event.preventDefault();
      event.stopPropagation();
      moveCursor(cursorIndex + (key === "j" ? 1 : -1));
      return;
    }
    if (key === "g") {
      event.preventDefault();
      event.stopPropagation();
      if (vimChordRef.current !== null) {
        globalThis.clearTimeout(vimChordRef.current);
        vimChordRef.current = null;
        moveCursor(0);
      } else {
        vimChordRef.current = globalThis.setTimeout(() => {
          vimChordRef.current = null;
        }, 900);
      }
      return;
    }
    if (key === "G") {
      event.preventDefault();
      event.stopPropagation();
      moveCursor(filtered.length - 1);
      return;
    }
    if (key === "l" || key === "Enter") {
      const page = filtered[cursorIndex];
      if (!page) return;
      event.preventDefault();
      event.stopPropagation();
      onSelect(page.id);
      return;
    }
    if (key === "h" && onVimDismiss) {
      event.preventDefault();
      event.stopPropagation();
      onVimDismiss();
      return;
    }
    if (key === "/" && searchable) {
      event.preventDefault();
      event.stopPropagation();
      listRef.current
        ?.closest<HTMLElement>("[data-question-directory]")
        ?.querySelector<HTMLInputElement>("[data-explore-page-search]")
        ?.focus();
    }
  };

  return (
    <Stack
      data-question-directory
      onKeyDown={onVimKeyDown}
      sx={{ minHeight: 0, height: "100%" }}
    >
      {searchable && (
        <TextField
          inputProps={{ "data-explore-page-search": "true" }}
          size="small"
          value={query}
          onChange={(event): void => setQuery(event.target.value)}
          placeholder="Search questions"
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <Search fontSize="small" />
                </InputAdornment>
              ),
            },
          }}
          sx={{ m: dense ? 1 : 2, mb: 1 }}
        />
      )}
      <Box
        sx={{
          flex: 1,
          minHeight: 0,
          position: "relative",
          // DetentSheet's body is itself scrollable. Keep this result region
          // geometrically contained so only the List below owns page scrolling.
          overflow: "hidden",
        }}
      >
        <Box
          role={showEarlierLoading && !query.trim() ? "status" : undefined}
          aria-live={showEarlierLoading && !query.trim() ? "polite" : undefined}
          aria-label={showEarlierLoading && !query.trim()
            ? "Loading earlier questions"
            : undefined}
          aria-hidden={!showEarlierLoading || !!query.trim()}
          sx={{
            position: "absolute",
            top: 8,
            left: "50%",
            transform: "translateX(-50%)",
            zIndex: 4,
            display: "flex",
            alignItems: "center",
            gap: 1,
            px: 1.5,
            minHeight: 34,
            borderRadius: 999,
            color: "text.secondary",
            bgcolor: (theme) => alpha(theme.palette.background.paper, 0.78),
            border: 1,
            borderColor: "divider",
            boxShadow: 2,
            backdropFilter: "blur(16px) saturate(1.25)",
            WebkitBackdropFilter: "blur(16px) saturate(1.25)",
            pointerEvents: "none",
            opacity: showEarlierLoading && !query.trim() ? 1 : 0,
            transition: "opacity 140ms ease",
          }}
        >
          <CircularProgress size={15} thickness={5} color="inherit" />
          <Typography variant="caption" sx={{ fontWeight: 650, whiteSpace: "nowrap" }}>
            Loading earlier questions
          </Typography>
        </Box>
        <List
          ref={(element): void => {
            listRef.current = element;
            if (listElementRef) listElementRef.current = element;
          }}
          dense={dense}
          tabIndex={vimNavigation ? 0 : undefined}
          autoFocus={vimNavigation}
          onScroll={updateBottomAffordance}
          sx={{
            position: "absolute",
            inset: 0,
            overflowY: "auto",
            px: dense ? 0.75 : 1,
            pt: 0.5,
            // Close is a true overlay during ordinary scrolling. Keep its
            // clearance inside the scroll content so the final page can still
            // rest fully above the island at the real end.
            pb: onDismiss
              ? "calc(76px + env(safe-area-inset-bottom, 0px))"
              : 0.5,
            overflowAnchor: "none",
          }}
        >
          {!query.trim() && !descending && (
            <Box
              ref={startSentinelRef}
              aria-hidden
              // MUI's sizing transform interprets numeric 1 as 100%.
              sx={{ height: "1px", pointerEvents: "none" }}
            />
          )}
          {filtered.map((page) => {
            const selected = page.id === currentId;
            const cursor = vimNavigation && page.id === cursorId;
            return (
              <ListItemButton
                key={page.id}
                data-page-id={page.id}
                ref={selected ? selectedRef : undefined}
                selected={selected}
                tabIndex={vimNavigation ? -1 : undefined}
                onClick={(): void => onSelect(page.id)}
                sx={{
                  minHeight: dense ? 44 : 52,
                  px: dense ? 1 : 1.25,
                  py: dense ? 0.625 : 0.875,
                  mb: 0.25,
                  borderRadius: 1.25,
                  alignItems: "center",
                  transition: "background-color 120ms ease",
                  "&.Mui-selected": {
                    bgcolor: (theme) => alpha(theme.palette.primary.main, 0.1),
                  },
                  ...(cursor && {
                    bgcolor: "action.hover",
                    boxShadow: (theme) =>
                      `inset 3px 0 0 ${theme.palette.primary.main}`,
                  }),
                  "&:not(.Mui-selected):hover": {
                    bgcolor: "action.hover",
                  },
                }}
              >
                <Typography
                  variant="caption"
                  color={selected ? "primary.main" : "text.secondary"}
                  sx={{
                    width: dense ? 30 : 38,
                    flexShrink: 0,
                    fontVariantNumeric: "tabular-nums",
                    lineHeight: 1,
                    opacity: selected ? 1 : 0.72,
                  }}
                >
                  {String(ordinalById.get(page.id) ?? 1)}
                </Typography>
                <ListItemText
                  primary={page.title}
                  sx={{ my: 0, minWidth: 0 }}
                  slotProps={{
                    primary: {
                      noWrap: true,
                      fontWeight: selected ? 650 : 500,
                      lineHeight: 1.35,
                    },
                  }}
                />
                {loadingPageId === page.id && (
                  <CircularProgress
                    aria-label="Loading question"
                    size={17}
                    thickness={5}
                    color="inherit"
                    sx={{ ml: 1, flexShrink: 0, color: "text.secondary" }}
                  />
                )}
              </ListItemButton>
            );
          })}
          {!query.trim() && descending && (
            <Box
              ref={startSentinelRef}
              aria-hidden
              sx={{ height: "1px", pointerEvents: "none" }}
            />
          )}
        </List>
      </Box>
    </Stack>
  );
}

export function ExploreTranscript(
  props: ExploreTranscriptProps,
): React.JSX.Element {
  const desktopWorkspace = useOptionalDesktopWorkspace();
  const pagination = useStoreSelector((snapshot) =>
    snapshot.pagination.get(props.sessionId)
  );
  const { pages, current, currentIndex, select } = usePages(
    props.sessionId,
    props.timeline,
  );
  const unresolvedQuestionRoot = current?.questionCount === 0 &&
    pagination?.reachedStart === false &&
    pagination.beforeSeq !== null;
  const requestedRootCursor = useRef<string | null>(null);
  const visibleItemKeys = useMemo(
    () => new Set(current?.itemKeys ?? []),
    [current?.itemKeys],
  );
  const pageIndex = useQuestionPageIndex(props.sessionId, pages.at(-1)?.id);
  const total = Math.max(pages.length, pageIndex.data?.total ?? 0);
  const provisionalOrdinal = Math.max(
    1,
    total - Math.max(0, pages.length - 1 - currentIndex),
  );
  const indexedPosition = indexedQuestionPagePosition(
    pageIndex.data?.pages ?? [],
    current?.id,
    provisionalOrdinal,
  );
  const indexedCurrentOrdinal = indexedPosition.ordinal;
  const currentOrdinal = indexedCurrentOrdinal ?? provisionalOrdinal;
  const atTail = indexedCurrentOrdinal === undefined
    ? currentIndex === pages.length - 1
    : indexedCurrentOrdinal === total;
  const directoryPages = useMemo(
    () => mergeQuestionPageDirectory(pageIndex.data?.pages ?? [], pages, total),
    [pageIndex.data?.pages, pages, total],
  );
  const rootRef = useRef<HTMLDivElement>(null);
  const [desktopDirectoryOpen, setDesktopDirectoryOpen] = useState(false);
  const [desktopDirectoryLoadingPageId, setDesktopDirectoryLoadingPageId] = useState<
    string | null
  >(null);
  const [footerLoadingPageId, setFooterLoadingPageId] = useState<string | null>(null);
  const loadedPrevious = currentIndex > 0 ? pages[currentIndex - 1] : undefined;
  const loadedNext = pages[currentIndex + 1];
  const footerPreviousId = indexedPosition.previousId ?? loadedPrevious?.id ?? null;
  const footerNextId = indexedPosition.nextId ?? loadedNext?.id ?? null;
  const footerPreviousQuestion = footerPreviousId
    ? directoryPages.find((page) => page.id === footerPreviousId)?.title ?? null
    : null;
  const footerNextQuestion = footerNextId
    ? directoryPages.find((page) => page.id === footerNextId)?.title ?? null
    : null;
  const navigateFooterPage = useCallback((id: string | null): void => {
    if (!id || footerLoadingPageId !== null) return;
    if (isQuestionPageLoaded(props.sessionId, id)) {
      select(id);
      return;
    }
    setFooterLoadingPageId(id);
    beginExplorePageLoading(props.sessionId);
    void loadQuestionPage(props.sessionId, id).then((loaded) => {
      setFooterLoadingPageId(null);
      if (loaded) select(id);
      else resolveExplorePageStart(props.sessionId);
    });
  }, [footerLoadingPageId, props.sessionId, select]);
  const goFooterPrevious = useCallback(
    (): void => navigateFooterPage(footerPreviousId),
    [footerPreviousId, navigateFooterPage],
  );
  const goFooterNext = useCallback(
    (): void => navigateFooterPage(footerNextId),
    [footerNextId, navigateFooterPage],
  );
  const desktopDirectoryReturnFocusRef = useRef<HTMLElement | null>(null);
  const closeDesktopDirectory = useCallback((restoreFocus = true): void => {
    setDesktopDirectoryOpen(false);
    const returnFocus = desktopDirectoryReturnFocusRef.current;
    desktopDirectoryReturnFocusRef.current = null;
    if (!restoreFocus) return;
    requestAnimationFrame(() => {
      if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
      else rootRef.current?.focus({ preventScroll: true });
    });
  }, []);
  const toggleDesktopDirectory = useCallback((): void => {
    if (desktopDirectoryOpen) {
      closeDesktopDirectory();
      return;
    }
    desktopDirectoryReturnFocusRef.current = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    setDesktopDirectoryOpen(true);
  }, [closeDesktopDirectory, desktopDirectoryOpen]);
  const selectDirectoryPage = useCallback((id: string, close: boolean): void => {
    if (desktopDirectoryLoadingPageId) return;
    if (isQuestionPageLoaded(props.sessionId, id)) {
      select(id);
      if (close) closeDesktopDirectory();
      return;
    }
    setDesktopDirectoryLoadingPageId(id);
    void loadQuestionPage(props.sessionId, id).then((loaded) => {
      setDesktopDirectoryLoadingPageId(null);
      if (!loaded) return;
      select(id);
      if (close) closeDesktopDirectory();
    });
  }, [
    closeDesktopDirectory,
    desktopDirectoryLoadingPageId,
    props.sessionId,
    select,
  ]);
  const {
    pageId: retainedPageId,
    pageStartId,
    pageLoadingId,
  } = useExploreSessionState(props.sessionId);
  const restoringPageRef = useRef<string | null>(null);
  const authoritativeTailId = authoritativeTailPageId(
    current,
    atTail,
    pageIndex.data?.pages ?? [],
  );
  const restorePageId = authoritativeTailId ?? retainedPageId;
  const restorePageLoaded = restorePageId === null ||
    isQuestionPageLoaded(props.sessionId, restorePageId);
  const restorePagePending = restorePageId !== null &&
    questionPageNeedsRestore(props.status, restorePageLoaded);

  useEffect(() => {
    if (
      !current || !pageIndex.data || pageIndex.loadingEarlier ||
      pageIndex.data.pages.some((page) => page.id === current.id) ||
      pageIndex.data.nextBeforeSeq === null
    ) return;
    void pageIndex.loadEarlier();
  }, [
    current,
    pageIndex.data,
    pageIndex.loadEarlier,
    pageIndex.loadingEarlier,
  ]);

  useEffect(() => {
    if (!restorePagePending || restorePageId === null) {
      restoringPageRef.current = null;
      return;
    }
    // The latest page is mutable only while its turn is active. Once the
    // session returns to idle, the authoritative index can safely repair a
    // retained projection that starts inside the completed answer.
    //
    // This request identity deliberately lives in a ref. Keeping it in state
    // made the effect cancel its own callback on the render caused by setting
    // "restoring", leaving the loaded page in cache behind an eternal skeleton
    // until the session was switched away and back.
    const requestKey = `${props.sessionId}:${restorePageId}`;
    if (restoringPageRef.current === requestKey) return;
    restoringPageRef.current = requestKey;
    void loadQuestionPage(props.sessionId, restorePageId).then((loaded) => {
      if (restoringPageRef.current !== requestKey) return;
      restoringPageRef.current = null;
      if (loaded && restorePageId !== retainedPageId) {
        setExplorePage(props.sessionId, restorePageId);
      } else if (!loaded && current) {
        // A deleted/corrupt retained page must not leave Page View behind an
        // eternal skeleton. Adopt the newest valid loaded page only then.
        setExplorePage(props.sessionId, current.id);
      }
    });
  }, [
    current?.id,
    props.sessionId,
    props.status,
    restorePageId,
    restorePageLoaded,
    restorePagePending,
    retainedPageId,
  ]);

  useEffect(() => () => {
    restoringPageRef.current = null;
  }, []);

  useEffect(() => {
    setDesktopDirectoryOpen(false);
    desktopDirectoryReturnFocusRef.current = null;
  }, [props.sessionId]);

  useEffect(() => {
    if (!desktopDirectoryOpen) return undefined;
    const frame = requestAnimationFrame(() => {
      rootRef.current
        ?.querySelector<HTMLInputElement>("[data-explore-page-search]")
        ?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [desktopDirectoryOpen]);

  useEffect(() => {
    if (!unresolvedQuestionRoot) {
      requestedRootCursor.current = null;
      return;
    }
    if (pagination?.loadingOlder || pagination?.beforeSeq === null) return;
    const requestKey = `${props.sessionId}:${String(pagination.beforeSeq)}`;
    if (requestedRootCursor.current === requestKey) return;
    requestedRootCursor.current = requestKey;
    // A retained live tail can begin in the middle of the latest answer. Fetch
    // question-aware history pages until its immutable user-message root is
    // present; rendering the provisional answer before then produces a false
    // "Earlier question" page and a large column-reverse blank region.
    const timer = window.setTimeout(() => {
      void loadPreviousQuestionPage(props.sessionId);
    }, 32);
    return () => window.clearTimeout(timer);
  }, [
    pagination?.beforeSeq,
    pagination?.loadingOlder,
    props.sessionId,
    unresolvedQuestionRoot,
  ]);

  useLayoutEffect(() => {
    setExploreAtTail(props.sessionId, atTail);
    return () => setExploreAtTail(props.sessionId, false);
  }, [atTail, props.sessionId]);

  const pageStartCurrentId = current?.id ?? null;
  const pageStartFirstKey = current?.itemKeys[0] ?? null;
  const pageStartIdentity = pageStartHandshakeIdentity(
    pageStartCurrentId,
    current?.itemKeys ?? [],
  );
  useLayoutEffect(() => {
    if (pageStartCurrentId === null || pageStartId !== pageStartCurrentId) return;
    // Page navigation is a reading action. Detach before positioning so a live
    // final page cannot have Transcript's streaming follow loop race this
    // page-start handshake back to the newest answer token. The shared navbar
    // button explicitly re-enables follow when the reader wants to catch up.
    setSticky(props.sessionId, false);
    globalThis.dispatchEvent(
      new CustomEvent("cowboy:explore-page-start", {
        detail: { sessionId: props.sessionId },
      }),
    );
    let frame = 0;
    let attempts = 0;
    let stableFrames = 0;
    let previousExtent = -1;
    let userInteracted = false;
    let observedScroller: HTMLElement | null = null;
    const relinquish = (): void => {
      userInteracted = true;
      cancelAnimationFrame(frame);
      resolveExplorePageStart(props.sessionId);
    };
    const observeUser = (scroller: HTMLElement): void => {
      if (observedScroller === scroller) return;
      observedScroller?.removeEventListener("pointerdown", relinquish);
      observedScroller?.removeEventListener("touchstart", relinquish);
      observedScroller?.removeEventListener("wheel", relinquish);
      observedScroller = scroller;
      scroller.addEventListener("pointerdown", relinquish, { passive: true });
      scroller.addEventListener("touchstart", relinquish, { passive: true });
      scroller.addEventListener("wheel", relinquish, { passive: true });
    };
    const positionAtStart = (): void => {
      if (userInteracted) return;
      const row = pageStartFirstKey
        ? rootRef.current?.querySelector<HTMLElement>(
          `[data-key="${CSS.escape(pageStartFirstKey)}"]`,
        )
        : null;
      const scroller = row?.parentElement;
      if (!scroller) {
        attempts += 1;
        // Transcript derives and swaps the filtered rows after this parent
        // changes page. Do not consume the start request against the outgoing
        // DOM; wait until the target question row actually exists.
        if (attempts < 90) frame = requestAnimationFrame(positionAtStart);
        else resolveExplorePageStart(props.sessionId);
        return;
      }
      if (!scroller) return;
      observeUser(scroller);
      // Transcript is a column-reverse scroller. WebKit's scrollIntoView()
      // treats block:start as the flex start (the newest edge), so a long
      // question page can reopen in the middle of its answer. The visual
      // beginning is the negative scroll extent.
      scroller.scrollTop = scroller.clientHeight - scroller.scrollHeight;
      const extent = scroller.scrollHeight - scroller.clientHeight;
      stableFrames = Math.abs(extent - previousExtent) < 0.5 ? stableFrames + 1 : 0;
      previousExtent = extent;
      attempts += 1;
      if ((attempts < 18 || stableFrames < 6) && attempts < 90) {
        // Keep the presentation overlay through the lazy Markdown/row commit,
        // then require a real stable-paint window. Ending on the first stable
        // frame exposed an unfinished page beneath a one-frame loading flash.
        frame = requestAnimationFrame(positionAtStart);
        return;
      }
      resolveExplorePageStart(props.sessionId);
    };
    frame = requestAnimationFrame(positionAtStart);
    return () => {
      cancelAnimationFrame(frame);
      observedScroller?.removeEventListener("pointerdown", relinquish);
      observedScroller?.removeEventListener("touchstart", relinquish);
      observedScroller?.removeEventListener("wheel", relinquish);
    };
    // A live page object is recreated for every streamed answer chunk. Key the
    // handshake only to its immutable root so streaming cannot repeatedly
    // restart the stable-paint counter and leave the loading veil visible
    // until the agent finishes.
  }, [pageStartIdentity, pageStartFirstKey, pageStartId, props.sessionId]);

  useEffect(() => {
    if (!props.desktop) return undefined;
    const root = rootRef.current;
    if (!root) return undefined;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        event.metaKey || event.ctrlKey || event.altKey || event.shiftKey
      ) return;
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (
        target?.matches("input, textarea, [contenteditable='true']") ||
        target?.closest("[contenteditable='true']")
      ) return;
      const key = event.code === "KeyJ"
        ? "j"
        : event.code === "KeyK"
        ? "k"
        : event.code === "KeyN"
        ? "n"
        : event.code === "BracketLeft"
        ? "["
        : event.code === "BracketRight"
        ? "]"
        : event.code === "Slash"
        ? "/"
        : event.key;
      if (key === "j" || key === "k") {
        event.preventDefault();
        event.stopPropagation();
        // Explore is still a reading surface. Keep j/k identical to History:
        // move by one visual line without replacing the entire question page.
        // Page changes remain explicit through the navigator and page controls.
        root.closest<HTMLElement>("[data-desktop-transcript-scroller]")
          ?.dispatchEvent(
            new CustomEvent("cowboy:desktop-transcript-nav", {
              detail: { action: key === "j" ? "line-down" : "line-up" },
            }),
          );
        return;
      }
      if (key === "/") {
        event.preventDefault();
        root.querySelector<HTMLInputElement>("[data-explore-page-search]")?.focus();
        return;
      }
      if (key === "[" || key === "]") {
        event.preventDefault();
        event.stopPropagation();
        if (key === "[") goFooterPrevious();
        else goFooterNext();
        return;
      }
      if (key === "n") {
        event.preventDefault();
        const prompt = document.querySelector<HTMLElement>(
          "[data-desktop-region='prompt.composer']",
        );
        prompt?.querySelector<HTMLElement>(
          "[data-vim-command-sink], .cm-content[contenteditable='true']",
        )?.focus({ preventScroll: true });
      }
    };
    root.addEventListener("keydown", onKeyDown);
    return () => root.removeEventListener("keydown", onKeyDown);
  }, [goFooterNext, goFooterPrevious, props.desktop]);

  if (!current && !props.loading) {
    return (
      <Stack
        alignItems="center"
        justifyContent="center"
        spacing={1}
        sx={{ flex: 1, color: "text.secondary" }}
      >
        <Typography variant="h6">No question pages yet</Typography>
        <Typography variant="body2">
          Ask the first question below to begin.
        </Typography>
      </Stack>
    );
  }

  return (
    <Box
      ref={rootRef}
      tabIndex={props.desktop ? 0 : undefined}
      sx={{
        flex: 1,
        minHeight: 0,
        display: "flex",
        position: "relative",
        outline: "none",
      }}
    >
      {props.desktop && (
        <DesktopQuestionDirectoryCommand
          available={pages.length > 0}
          toggle={toggleDesktopDirectory}
        />
      )}
      {props.desktop && pages.length > 0 && (
        <DesktopModal
          open={desktopDirectoryOpen}
          onClose={closeDesktopDirectory}
          title="Question Navigator"
          description={`${String(total)} questions · newest first · j/k move · l/Enter open`}
          icon={<ListAltOutlined color="primary" />}
          width={620}
        >
          <Box
            component="nav"
            aria-label="Question pages"
            data-desktop-region="conversation.questions"
            sx={{ height: "min(640px, calc(100vh - 190px))", minHeight: 320 }}
          >
            <PageList
              active={desktopDirectoryOpen}
              dense
              descending
              vimNavigation
              pages={directoryPages}
              currentId={current?.id ?? null}
              firstOrdinal={Math.max(1, total - directoryPages.length + 1)}
              hasEarlier={pageIndex.data?.nextBeforeSeq !== null}
              loadingEarlier={pageIndex.loadingEarlier}
              loadingPageId={desktopDirectoryLoadingPageId}
              onReachStart={(): void => void pageIndex.loadEarlier()}
              onVimDismiss={closeDesktopDirectory}
              onSelect={(id): void => selectDirectoryPage(id, true)}
            />
          </Box>
        </DesktopModal>
      )}
      {props.desktop && desktopWorkspace?.productMode === "reading" &&
        desktopWorkspace.readingSidebarOpen && pages.length > 0 && (
        <Box
          component="nav"
          aria-label="Question pages"
          data-reading-page-list
          sx={{
            position: "relative",
            width: "clamp(240px, 20vw, 320px)",
            flexShrink: 0,
            minHeight: 0,
            borderRight: 1,
            borderColor: "divider",
            bgcolor: (theme) => alpha(theme.palette.background.paper, 0.42),
          }}
        >
          <PageList
            active
            dense
            descending
            vimNavigation
            searchable={false}
            pages={directoryPages}
            currentId={current?.id ?? null}
            firstOrdinal={Math.max(1, total - directoryPages.length + 1)}
            hasEarlier={pageIndex.data?.nextBeforeSeq !== null}
            loadingEarlier={pageIndex.loadingEarlier}
            loadingPageId={desktopDirectoryLoadingPageId}
            onReachStart={(): void => void pageIndex.loadEarlier()}
            onSelect={(id): void => selectDirectoryPage(id, false)}
          />
        </Box>
      )}
      <Stack sx={{ flex: 1, minWidth: 0, minHeight: 0, position: "relative" }}>
        {unresolvedQuestionRoot || restorePagePending
          ? (
            <Stack
              role="status"
              aria-label="Loading question history"
              spacing={2.25}
              sx={{
                flex: 1,
                minHeight: 0,
                px: { xs: 2, md: 3 },
                pt: { xs: 3, md: 4 },
                overflow: "hidden",
              }}
            >
              <Skeleton variant="rounded" width="38%" height="2.75em" sx={{ alignSelf: "flex-end" }} />
              <Stack spacing={0.75}>
                <Skeleton variant="text" width="92%" />
                <Skeleton variant="text" width="74%" />
                <Skeleton variant="text" width="84%" />
              </Stack>
              <Skeleton variant="rounded" width="100%" height="8em" />
              <Stack spacing={0.75}>
                <Skeleton variant="text" width="88%" />
                <Skeleton variant="text" width="68%" />
              </Stack>
            </Stack>
          )
          : (
            <Transcript
              key={`${props.sessionId}:${current?.id ?? ""}`}
              desktopNavigation={props.desktop}
              historyPaging="page"
              sessionId={props.sessionId}
              timeline={props.timeline}
              status={props.status}
              provider={props.provider}
              cwd={props.cwd}
              loading={props.loading}
              connected={props.connected}
              topInset={props.topInset}
              bottomInset={props.bottomInset}
              onScrollableChange={props.onScrollableChange}
              visibleItemKeys={visibleItemKeys}
              pageFooter={current
                ? (
                  <PageTurnFooter
                    currentOrdinal={currentOrdinal}
                    total={total}
                    previousDisabled={footerPreviousId === null}
                    nextDisabled={footerNextId === null}
                    previousQuestion={footerPreviousQuestion}
                    nextQuestion={footerNextQuestion}
                    loadingPrevious={footerLoadingPageId !== null &&
                      footerLoadingPageId === footerPreviousId}
                    loadingNext={footerLoadingPageId !== null &&
                      footerLoadingPageId === footerNextId}
                    onPrevious={goFooterPrevious}
                    onNext={goFooterNext}
                    desktop={props.desktop}
                  />
                )
                : undefined}
              pageId={current?.id}
              liveTail={atTail}
              shortContentAtTop
            />
          )}
        {pageLoadingId !== null && (
          <Stack
            role="status"
            aria-live="polite"
            aria-label="Loading page"
            spacing={2.25}
            sx={{
              position: "absolute",
              zIndex: 4,
              inset: 0,
              minHeight: 0,
              px: { xs: 2, md: 3 },
              pt: { xs: 3, md: 4 },
              overflow: "hidden",
              bgcolor: "background.default",
            }}
          >
            <Skeleton variant="rounded" width="38%" height="2.75em" sx={{ alignSelf: "flex-end" }} />
            <Stack spacing={0.75}>
              <Skeleton variant="text" width="92%" />
              <Skeleton variant="text" width="74%" />
              <Skeleton variant="text" width="84%" />
            </Stack>
            <Skeleton variant="rounded" width="100%" height="8em" />
            <Stack spacing={0.75}>
              <Skeleton variant="text" width="88%" />
              <Skeleton variant="text" width="68%" />
            </Stack>
          </Stack>
        )}
      </Stack>
    </Box>
  );
}

export function MobilePageDock({
  sessionId,
  composeOpen,
  onComposeToggle,
}: {
  sessionId: string;
  composeOpen: boolean;
  onComposeToggle: (knownPageIds: string[]) => void;
}): React.JSX.Element {
  const timeline = useStoreSelector((snapshot) =>
    snapshot.timelines.get(sessionId) ?? EMPTY_TIMELINE
  );
  const pagination = useStoreSelector((snapshot) =>
    snapshot.pagination.get(sessionId)
  );
  const { pages, current, currentIndex, navigate } = usePages(sessionId, timeline);
  const pageIndex = useQuestionPageIndex(sessionId, pages.at(-1)?.id);
  const [open, setOpen] = useState(false);
  const [loadingDirectoryPageId, setLoadingDirectoryPageId] = useState<string | null>(null);
  const [loadingAdjacentPageId, setLoadingAdjacentPageId] = useState<string | null>(null);
  const [pageDirectoryAwayFromBottom, setPageDirectoryAwayFromBottom] = useState(false);
  const pageDirectoryListRef = useRef<HTMLUListElement | null>(null);
  const [showPageTop, setShowPageTop] = useState(false);
  const [pendingPrevious, setPendingPrevious] = useState<{
    anchorItemKey: string;
    requestedBeforeSeq: number | null;
    requestComplete: boolean;
  } | null>(null);
  const previous = currentIndex > 0 && pages[currentIndex - 1]?.questionCount
    ? pages[currentIndex - 1]
    : undefined;
  const next = pages[currentIndex + 1];
  const knownPageIds = pages.map((page) => page.id);
  const composeToggleTap = useReliableTouchTap<HTMLButtonElement>(() =>
    onComposeToggle(knownPageIds)
  );
  const hasEarlierHistory = pagination?.reachedStart === false;
  const loadingEarlier = pagination?.loadingOlder === true;
  const total = Math.max(pages.length, pageIndex.data?.total ?? 0);
  const indexedPages = pageIndex.data?.pages ?? [];
  const indexedPosition = indexedQuestionPagePosition(indexedPages, current?.id);
  const indexedPrevious = indexedPosition.previousId
    ? indexedPages.find((page) => page.id === indexedPosition.previousId)
    : undefined;
  const indexedNext = indexedPosition.nextId
    ? indexedPages.find((page) => page.id === indexedPosition.nextId)
    : undefined;
  const loadingPrevious = loadingEarlier || pendingPrevious !== null ||
    loadingAdjacentPageId === indexedPrevious?.id;
  const loadingNext = loadingAdjacentPageId === indexedNext?.id;
  const onlyCompletePage = total <= 1 && !hasEarlierHistory;
  const directoryPages = useMemo(
    () => mergeQuestionPageDirectory(pageIndex.data?.pages ?? [], pages, total),
    [pageIndex.data?.pages, pages, total],
  );
  const currentOrdinal = indexedPosition.ordinal !== undefined
    ? indexedPosition.ordinal
    : Math.max(
      1,
      total - Math.max(0, pages.length - 1 - currentIndex),
    );

  useEffect(() => {
    if (
      !current || !pageIndex.data || pageIndex.loadingEarlier ||
      pageIndex.data.pages.some((page) => page.id === current.id) ||
      pageIndex.data.nextBeforeSeq === null
    ) return;
    void pageIndex.loadEarlier();
  }, [
    current,
    pageIndex.data,
    pageIndex.loadEarlier,
    pageIndex.loadingEarlier,
  ]);

  const closePageDirectory = useCallback((): void => setOpen(false), []);
  const scrollPageDirectoryToLatest = useCallback((): void => {
    const list = pageDirectoryListRef.current;
    if (!list) return;
    // DetentSheet is transformed; direct nested-scroll assignment is reliable
    // on iOS where smooth Element.scrollTo() can be dropped.
    list.scrollTop = Math.max(0, list.scrollHeight - list.clientHeight);
    setPageDirectoryAwayFromBottom(false);
  }, []);

  useEffect(() => {
    let frame = 0;
    let scroller: HTMLElement | null = null;
    const update = (): void => {
      if (!scroller) return;
      const visualStart = scroller.clientHeight - scroller.scrollHeight;
      setShowPageTop(
        scroller.scrollTop - visualStart > 8,
      );
    };
    const bind = (): void => {
      scroller = document.querySelector<HTMLElement>(
        `[data-transcript-session="${CSS.escape(sessionId)}"]`,
      );
      if (!scroller) {
        frame = requestAnimationFrame(bind);
        return;
      }
      update();
      scroller.addEventListener("scroll", update, { passive: true });
    };
    frame = requestAnimationFrame(bind);
    return () => {
      cancelAnimationFrame(frame);
      scroller?.removeEventListener("scroll", update);
    };
  }, [current?.id, sessionId]);

  const scrollPageToTop = (): void => {
    if (!current) return;
    // Reuse the page-start handshake so WebKit cannot race this navigation
    // against Transcript's deferred column-reverse layout.
    const pageId = current.id;
    resolveExplorePageStart(sessionId);
    requestAnimationFrame(() => navigateExplorePage(sessionId, pageId));
    setShowPageTop(false);
  };

  const navigateIndexedPage = (id: string): void => {
    if (loadingAdjacentPageId !== null) return;
    if (isQuestionPageLoaded(sessionId, id)) {
      navigate(id);
      return;
    }
    setLoadingAdjacentPageId(id);
    beginExplorePageLoading(sessionId);
    void loadQuestionPage(sessionId, id).then((loaded) => {
      setLoadingAdjacentPageId(null);
      if (loaded) {
        navigate(id);
      } else {
        resolveExplorePageStart(sessionId);
      }
    });
  };

  const goPrevious = (): void => {
    if (indexedPrevious) {
      navigateIndexedPage(indexedPrevious.id);
      return;
    }
    if (previous) {
      navigate(previous.id);
      return;
    }
    if (!hasEarlierHistory || loadingPrevious || !current) return;
    // One answer can span several bounded HTTP history pages. Keep loading
    // under this single user action until the preceding question boundary is
    // found, rather than making the reader tap once per transport page.
    // The question root is immutable while its answer streams. Using the last
    // answer item here made this anchor change underneath the pending history
    // request, which could repeatedly restart navigation and freeze WebKit.
    const anchorItemKey = current.id;
    if (!anchorItemKey) return;
    setPendingPrevious({
      anchorItemKey,
      requestedBeforeSeq: null,
      requestComplete: true,
    });
    beginExplorePageLoading(sessionId);
  };

  useEffect(() => {
    if (!pendingPrevious || loadingEarlier) return;
    // A bounded tail can begin midway through the current answer. Its
    // provisional page id changes when the real user prompt is loaded, so
    // anchor navigation to an immutable item inside that answer instead.
    const loadedPrevious = completePageBeforeItem(
      pages,
      pendingPrevious.anchorItemKey,
    );
    if (loadedPrevious) {
      navigate(loadedPrevious.id);
      setPendingPrevious(null);
      return;
    }
    const beforeSeq = pagination?.beforeSeq ?? null;
    if (!hasEarlierHistory || beforeSeq === null) {
      setPendingPrevious(null);
      resolveExplorePageStart(sessionId);
      return;
    }
    if (!pendingPrevious.requestComplete) return;
    if (beforeSeq === pendingPrevious.requestedBeforeSeq) {
      // The request failed without advancing its immutable cursor.
      setPendingPrevious(null);
      resolveExplorePageStart(sessionId);
      return;
    }
    setPendingPrevious({
      ...pendingPrevious,
      requestedBeforeSeq: beforeSeq,
      requestComplete: false,
    });
    // Yield between transport pages so a long answer cannot monopolize
    // WKWebView's main thread while React derives the growing page window.
    window.setTimeout(() => {
      void loadPreviousQuestionPage(sessionId).finally(() => {
        setPendingPrevious((value) =>
          value?.requestedBeforeSeq === beforeSeq
            ? { ...value, requestComplete: true }
            : value
        );
      });
    }, 32);
  }, [
    hasEarlierHistory,
    loadingEarlier,
    navigate,
    pages,
    pagination?.beforeSeq,
    pendingPrevious,
    sessionId,
  ]);

  return (
    <>
      <Paper
        component="nav"
        aria-label="Question pages"
        variant="outlined"
        elevation={0}
        sx={{
          minHeight: 52,
          mx: "max(env(safe-area-inset-left), 12px)",
          mr: "max(env(safe-area-inset-right), 12px)",
          mt: 1.5,
          mb: 0.75,
          px: "0.5em",
          display: "grid",
          gridTemplateColumns: "1fr auto 1fr",
          alignItems: "center",
          borderColor: "divider",
          borderRadius: 2,
          overflow: "hidden",
          bgcolor: "transparent",
        }}
      >
        <Stack
          direction="row"
          gap={0.75}
          sx={{
            justifySelf: "start",
            "& > span": { display: "flex" },
          }}
        >
          {!onlyCompletePage && (
            <>
              <Tooltip title={hasEarlierHistory && !indexedPrevious && !previous
                ? "Load earlier questions"
                : "Previous page"}
              >
                <span>
                  <IconButton
                    aria-label={hasEarlierHistory && !indexedPrevious && !previous
                      ? "Load earlier questions"
                      : "Previous page"}
                    disabled={(!indexedPrevious && !previous && !hasEarlierHistory) ||
                      loadingPrevious || loadingNext}
                    onClick={goPrevious}
                    sx={{
                      width: "max(40px, 2.5rem)",
                      height: "max(40px, 2.5rem)",
                      border: 1,
                      borderColor: "divider",
                      borderRadius: "50%",
                      bgcolor: "action.hover",
                      "&:active": { bgcolor: "action.selected" },
                      "&.Mui-disabled": {
                        color: "text.disabled",
                        borderColor: "divider",
                        bgcolor: "action.disabledBackground",
                        opacity: 0.58,
                      },
                    }}
                  >
                    {loadingPrevious
                      ? <CircularProgress size="1.0625rem" />
                      : <ChevronLeft sx={{ fontSize: "1.25em" }} />}
                  </IconButton>
                </span>
              </Tooltip>
              <Tooltip title="Next page">
                <span>
                  <IconButton
                    aria-label="Next page"
                    disabled={(!indexedNext && !next) || loadingAdjacentPageId !== null}
                    onClick={(): void => {
                      if (indexedNext) navigateIndexedPage(indexedNext.id);
                      else if (next) navigate(next.id);
                    }}
                    sx={{
                      width: "max(40px, 2.5rem)",
                      height: "max(40px, 2.5rem)",
                      border: 1,
                      borderColor: "divider",
                      borderRadius: "50%",
                      bgcolor: "action.hover",
                      "&:active": { bgcolor: "action.selected" },
                      "&.Mui-disabled": {
                        color: "text.disabled",
                        borderColor: "divider",
                        bgcolor: "action.disabledBackground",
                        opacity: 0.58,
                      },
                    }}
                  >
                    {loadingNext
                      ? <CircularProgress size="1.0625rem" />
                      : <ChevronRight sx={{ fontSize: "1.25em" }} />}
                  </IconButton>
                </span>
              </Tooltip>
            </>
          )}
        </Stack>
        <Button
          aria-label="Open question pages"
          onClick={(): void => setOpen(true)}
          startIcon={<ListAltOutlined />}
          sx={{
            minWidth: "7.25em",
            minHeight: "max(40px, 2.5rem)",
            px: "1em",
            border: 1,
            borderColor: "divider",
            borderRadius: "1.25rem",
            bgcolor: "action.hover",
            textTransform: "none",
            color: "text.primary",
            fontSize: "1rem",
            "& .MuiButton-startIcon": { mr: "0.6em" },
            "& .MuiButton-startIcon > *:nth-of-type(1)": {
              fontSize: "1.35em",
            },
            "&:active": { bgcolor: "action.selected" },
          }}
        >
          {pageIndex.loading && pageIndex.data === null
            ? (
              <Skeleton
                aria-label="Loading page count"
                variant="text"
                width="3.8em"
                sx={{ fontSize: "0.75em" }}
              />
            )
            : (
              <Typography
                variant="caption"
                noWrap
                sx={{ fontWeight: 750, fontVariantNumeric: "tabular-nums" }}
              >
                {pages.length === 0
                  ? "Pages"
                  : `${String(currentOrdinal)} / ${String(total)}${
                    pageIndex.data?.exact === false ? "+" : ""
                  }`}
              </Typography>
            )}
        </Button>
        <Stack
          direction="row"
          gap={0.75}
          alignItems="center"
          sx={{
            justifySelf: "end",
            "& > span": { display: "flex" },
          }}
        >
          <Tooltip title={showPageTop ? "Back to page top" : "At page top"}>
            <span>
              <IconButton
                aria-label="Back to page top"
                disabled={!showPageTop}
                onClick={scrollPageToTop}
                sx={{
                  width: "max(40px, 2.5rem)",
                  height: "max(40px, 2.5rem)",
                  border: 1,
                  borderColor: "divider",
                  borderRadius: "50%",
                  bgcolor: "action.hover",
                  "&:active": { bgcolor: "action.selected" },
                  "&.Mui-disabled": {
                    color: "text.disabled",
                    borderColor: "divider",
                    bgcolor: "action.disabledBackground",
                    opacity: 0.58,
                  },
                }}
              >
                <ArrowUpward sx={{ fontSize: "1.25em" }} />
              </IconButton>
            </span>
          </Tooltip>
          <Tooltip title={composeOpen ? "Close question editor" : "Open question editor"}>
            <IconButton
              color="primary"
              aria-label={composeOpen ? "Close question editor" : "Open question editor"}
              aria-pressed={composeOpen}
              {...composeToggleTap}
              sx={{
                width: "max(40px, 2.5rem)",
                height: "max(40px, 2.5rem)",
                border: 1,
                borderColor: "divider",
                borderRadius: "50%",
                bgcolor: "action.hover",
                "&:active": { bgcolor: "action.selected" },
              }}
            >
              {composeOpen
                ? <Close sx={{ fontSize: "1.25em" }} />
                : <EditOutlined sx={{ fontSize: "1.25em" }} />}
            </IconButton>
          </Tooltip>
        </Stack>
      </Paper>
      <DetentSheet
        open={open}
        onClose={closePageDirectory}
        ariaLabel="Question pages"
        frosted
        cover
        footer={
          <MobileSheetActionGroup
            actions={[
              {
                key: "close",
                label: "Close",
                onPress: closePageDirectory,
                icon: <Close aria-hidden fontSize="small" sx={{ transform: "translate(-0.75px, -0.5px)" }} />,
              },
              {
                key: "latest",
                label: "Scroll to latest question",
                onPress: scrollPageDirectoryToLatest,
                visible: pageDirectoryAwayFromBottom,
                icon: <ArrowDownward aria-hidden sx={{ fontSize: "1.25em" }} />,
              },
            ]}
          />
        }
        footerOverlay
      >
        <Box
          sx={{
            // Consume DetentSheet's generic overlay-footer clearance so page
            // rows extend behind the floating Close island. PageList owns the
            // equivalent clearance inside its scrollable content instead.
            height: "calc(100% + 76px + env(safe-area-inset-bottom, 0px))",
            mb: "calc(-76px - env(safe-area-inset-bottom, 0px))",
            display: "flex",
            flexDirection: "column",
            minHeight: 0,
          }}
        >
          <Stack
            direction="row"
            alignItems="center"
            sx={{
              minHeight: 52,
              px: 2,
              pt: 1,
              flexShrink: 0,
              overflow: "visible",
            }}
          >
            <Typography
              variant="h6"
              sx={{ fontWeight: 750, lineHeight: 1.25, overflow: "visible" }}
            >
              Pages
            </Typography>
            {pageIndex.loading && pageIndex.data === null
              ? (
                <Skeleton
                  aria-label="Loading page count"
                  variant="text"
                  width="2.5em"
                  sx={{ ml: 1, fontSize: "0.75rem" }}
                />
              )
              : (
                <Typography variant="caption" color="text.secondary" sx={{ ml: 1 }}>
                  {`${String(total)}${pageIndex.data?.exact === false ? "+" : ""}`}
                </Typography>
              )}
          </Stack>
          <Box sx={{ flex: 1, minHeight: 0 }}>
            <PageList
              active={open}
              searchable={false}
              listElementRef={pageDirectoryListRef}
              onAwayFromBottomChange={setPageDirectoryAwayFromBottom}
              pages={directoryPages}
              currentId={current?.id ?? null}
              firstOrdinal={Math.max(1, total - directoryPages.length + 1)}
              hasEarlier={pageIndex.data?.nextBeforeSeq !== null}
              loadingEarlier={pageIndex.loadingEarlier}
              loadingPageId={loadingDirectoryPageId}
              onReachStart={(): void => void pageIndex.loadEarlier()}
              onDismiss={closePageDirectory}
              onSelect={(id): void => {
                if (loadingDirectoryPageId) return;
                if (isQuestionPageLoaded(sessionId, id)) {
                  navigate(id);
                  setOpen(false);
                  return;
                }
                setLoadingDirectoryPageId(id);
                void loadQuestionPage(sessionId, id).then((loaded) => {
                  setLoadingDirectoryPageId(null);
                  if (!loaded) return;
                  navigate(id);
                  setOpen(false);
                });
              }}
            />
          </Box>
        </Box>
      </DetentSheet>
    </>
  );
}
