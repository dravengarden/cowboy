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
  FloatingActionIsland,
  MobileSheetDismiss,
} from "../_shell";
import { derive } from "../derive";
import type { Envelope, Status } from "../protocol";
import {
  loadPreviousQuestionPage,
  useStoreSelector,
} from "../store";
import { setSticky } from "../stickyStore";
import { Transcript } from "../Transcript";
import {
  setExplorePage,
  setExploreAtTail,
  navigateExplorePage,
  resolveExplorePageStart,
  resolveProjectionAnchor,
  resolveExploreFollowUp,
  useExploreSessionState,
} from "./exploreStore";
import {
  completePageBeforeItem,
  deriveQuestionPages,
  groupQuestionPages,
  pageContainingItemKey,
  type QuestionPage,
} from "./questionPages";

const EMPTY_TIMELINE: Envelope[] = [];

function useQuestionPageIndex(
  sessionId: string,
  revisionKey: string | undefined,
): {
  data: { total: number; exact: boolean } | null;
  loading: boolean;
} {
  const [state, setState] = useState<{
    data: { total: number; exact: boolean } | null;
    loading: boolean;
    sessionId: string;
  }>({ data: null, loading: true, sessionId });
  const data = state.sessionId === sessionId ? state.data : null;
  useEffect(() => {
    const controller = new AbortController();
    setState((current) => ({
      data: current.sessionId === sessionId ? current.data : null,
      loading: true,
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
        return response.json() as Promise<{ total: number; exact: boolean }>;
      })
      .then((next) => setState({ data: next, loading: false, sessionId }))
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
  return { data, loading: state.loading || state.sessionId !== sessionId };
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
    if (current && current.id !== pageId) setExplorePage(sessionId, current.id);
  }, [current?.id, pageId, sessionId, transitionAnchorKey]);

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
  onReachStart,
  onDismiss,
  active = true,
}: {
  pages: QuestionPage[];
  currentId: string | null;
  onSelect: (id: string) => void;
  dense?: boolean;
  firstOrdinal?: number;
  hasEarlier?: boolean;
  loadingEarlier?: boolean;
  onReachStart?: (() => void) | undefined;
  onDismiss?: (() => void) | undefined;
  active?: boolean;
}): React.JSX.Element {
  const [query, setQuery] = useState("");
  const [awayFromBottom, setAwayFromBottom] = useState(false);
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
  const ordinalById = useMemo(
    () => new Map(pages.map((page, index) => [page.id, firstOrdinal + index])),
    [firstOrdinal, pages],
  );
  const filtered = query.trim()
    ? pages.filter((page) =>
      page.title.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())
    )
    : pages;

  const updateBottomAffordance = useCallback((): void => {
    const list = listRef.current;
    if (!list || !onDismiss) {
      setAwayFromBottom(false);
      return;
    }
    setAwayFromBottom(
      list.scrollHeight - list.scrollTop - list.clientHeight > 48,
    );
    // One upward approach loads one batch. Once prepending has moved the
    // viewport clear of the boundary, arm the next deliberate approach.
    if (list.scrollTop > 120) earlierRequestArmedRef.current = true;
  }, [onDismiss]);

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
    if (list) {
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
  }, [onReachStart, pages.length, showEarlierLoading]);

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
    // A tall phone/tablet can still have no scroll range after one question
    // boundary lands. Re-arm before the observer is recreated so it keeps
    // filling by visible directory pages; otherwise the user cannot make a
    // second upward approach because there is literally nothing to scroll.
    if (list.scrollHeight <= list.clientHeight + 1) {
      earlierRequestArmedRef.current = true;
    }
    prependAnchorRef.current = null;
  }, [pages.length]);

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
        // Start fetching just before the first row reaches the sheet edge.
        rootMargin: "72px 0px 0px",
        threshold: 0,
      },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasEarlier, loadingEarlier, query, requestEarlier]);

  return (
    <Stack sx={{ minHeight: 0, height: "100%" }}>
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
          ref={listRef}
          dense={dense}
          onScroll={updateBottomAffordance}
          sx={{
            position: "absolute",
            inset: 0,
            overflowY: "auto",
            px: dense ? 0.75 : 1,
            pt: 0.5,
            pb: 0.5,
            overflowAnchor: "none",
          }}
        >
          {!query.trim() && (
            <Box
              ref={startSentinelRef}
              aria-hidden
              // MUI's sizing transform interprets numeric 1 as 100%.
              sx={{ height: "1px", pointerEvents: "none" }}
            />
          )}
          {filtered.map((page) => {
            const selected = page.id === currentId;
            return (
              <ListItemButton
                key={page.id}
                data-page-id={page.id}
                ref={selected ? selectedRef : undefined}
                selected={selected}
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
              </ListItemButton>
            );
          })}
        </List>
        {onDismiss && (
          <Box
            aria-label="Page directory actions"
            sx={{
              position: "absolute",
              insetInline: 0,
              bottom: "max(10px, env(safe-area-inset-bottom, 0px))",
              zIndex: 3,
              height: 54,
              pointerEvents: "none",
            }}
          >
            {awayFromBottom && (
              <Box
                sx={{
                  position: "absolute",
                  right: "max(14px, env(safe-area-inset-right, 0px))",
                  pointerEvents: "auto",
                }}
              >
                <FloatingActionIsland maxWidth={54}>
                  <IconButton
                    aria-label="Scroll to latest question"
                    onClick={(): void => {
                      listRef.current?.scrollTo({
                        top: listRef.current.scrollHeight,
                        behavior: "smooth",
                      });
                    }}
                    sx={{ width: 46, height: 46 }}
                  >
                    <ArrowDownward sx={{ fontSize: "1.25em" }} />
                  </IconButton>
                </FloatingActionIsland>
              </Box>
            )}
          </Box>
        )}
      </Box>
    </Stack>
  );
}

export function ExploreTranscript(
  props: ExploreTranscriptProps,
): React.JSX.Element {
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
  const atTail = currentIndex === pages.length - 1;
  const pageIndex = useQuestionPageIndex(props.sessionId, pages.at(-1)?.id);
  const total = Math.max(pages.length, pageIndex.data?.total ?? 0);
  const rootRef = useRef<HTMLDivElement>(null);
  const { pageStartId } = useExploreSessionState(props.sessionId);

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

  useLayoutEffect(() => {
    if (!current || pageStartId !== current.id) return;
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
    let positionedFrames = 0;
    const positionAtStart = (): void => {
      const firstKey = current.itemKeys[0];
      const row = firstKey
        ? rootRef.current?.querySelector<HTMLElement>(
          `[data-key="${CSS.escape(firstKey)}"]`,
        )
        : null;
      const scroller = row?.parentElement;
      if (!scroller && attempts++ < 12) {
        // Transcript derives and swaps the filtered rows after this parent
        // changes page. Do not consume the start request against the outgoing
        // DOM; wait until the target question row actually exists.
        frame = requestAnimationFrame(positionAtStart);
        return;
      }
      if (!scroller) return;
      // Transcript is a column-reverse scroller. WebKit's scrollIntoView()
      // treats block:start as the flex start (the newest edge), so a long
      // question page can reopen in the middle of its answer. The visual
      // beginning is the negative scroll extent.
      scroller.scrollTop = scroller.clientHeight - scroller.scrollHeight;
      if (positionedFrames++ < 11) {
        // Transcript also initializes its column-reverse position in a layout
        // effect and may commit a deferred derived row set after several
        // paints. Reassert through that hand-off so cached page switches cannot
        // be overwritten back to the newest edge.
        frame = requestAnimationFrame(positionAtStart);
        return;
      }
      resolveExplorePageStart(props.sessionId);
    };
    frame = requestAnimationFrame(positionAtStart);
    return () => cancelAnimationFrame(frame);
  }, [current, pageStartId, props.sessionId]);

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
        : event.code === "Slash"
        ? "/"
        : event.key;
      if (key === "j" || key === "k") {
        event.preventDefault();
        const next = Math.max(
          0,
          Math.min(pages.length - 1, currentIndex + (key === "j" ? 1 : -1)),
        );
        const page = pages[next];
        if (page) select(page.id);
        return;
      }
      if (key === "/") {
        event.preventDefault();
        root.querySelector<HTMLInputElement>("[data-explore-page-search]")?.focus();
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
  }, [currentIndex, pages, props.desktop, select]);

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
        outline: "none",
      }}
    >
      {props.desktop && pages.length > 0 && (
        <>
          <Box
            component="nav"
            aria-label="Question pages"
            sx={{
              width: "clamp(210px, 22%, 320px)",
              flexShrink: 0,
              minHeight: 0,
              borderRight: 1,
              borderColor: "divider",
              bgcolor: (theme) => alpha(theme.palette.background.paper, 0.3),
            }}
          >
            <PageList
              dense
              pages={pages}
              currentId={current?.id ?? null}
              firstOrdinal={Math.max(1, total - pages.length + 1)}
              onSelect={select}
            />
          </Box>
        </>
      )}
      <Stack sx={{ flex: 1, minWidth: 0, minHeight: 0, position: "relative" }}>
        {unresolvedQuestionRoot
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
              pageId={current?.id}
              liveTail={atTail}
              shortContentAtTop
            />
          )}
        {props.desktop && (
          <Stack
            direction="row"
            alignItems="center"
            justifyContent="flex-end"
            spacing={1.25}
            sx={{
              minHeight: 30,
              px: 1.5,
              borderTop: 1,
              borderColor: "divider",
              color: "text.secondary",
              flexShrink: 0,
            }}
          >
            <Typography variant="caption"><b>J/K</b> Page</Typography>
            <Typography variant="caption"><b>/</b> Search</Typography>
            <Typography variant="caption"><b>N</b> New question</Typography>
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
  const hasEarlierHistory = pagination?.reachedStart === false;
  const loadingEarlier = pagination?.loadingOlder === true;
  const loadingPrevious = loadingEarlier || pendingPrevious !== null;
  const onlyCompletePage = pages.length <= 1 && !hasEarlierHistory;
  const total = Math.max(pages.length, pageIndex.data?.total ?? 0);
  const currentOrdinal = Math.max(
    1,
    total - Math.max(0, pages.length - 1 - currentIndex),
  );
  const closePageDirectory = useCallback((): void => setOpen(false), []);

  useEffect(() => {
    let frame = 0;
    let scroller: HTMLElement | null = null;
    const update = (): void => {
      if (!scroller) return;
      const visualStart = scroller.clientHeight - scroller.scrollHeight;
      setShowPageTop(
        scroller.scrollTop - visualStart > scroller.clientHeight * 0.75,
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

  const goPrevious = (): void => {
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
      return;
    }
    if (!pendingPrevious.requestComplete) return;
    if (beforeSeq === pendingPrevious.requestedBeforeSeq) {
      // The request failed without advancing its immutable cursor.
      setPendingPrevious(null);
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
              <Tooltip title={hasEarlierHistory && !previous
                ? "Load earlier questions"
                : "Previous page"}
              >
                <span>
                  <IconButton
                    aria-label={hasEarlierHistory && !previous
                      ? "Load earlier questions"
                      : "Previous page"}
                    disabled={(!previous && !hasEarlierHistory) || loadingPrevious}
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
                        borderColor: "transparent",
                        bgcolor: "transparent",
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
                    disabled={!next}
                    onClick={(): void => next && navigate(next.id)}
                    sx={{
                      width: "max(40px, 2.5rem)",
                      height: "max(40px, 2.5rem)",
                      border: 1,
                      borderColor: "divider",
                      borderRadius: "50%",
                      bgcolor: "action.hover",
                      "&:active": { bgcolor: "action.selected" },
                      "&.Mui-disabled": {
                        borderColor: "transparent",
                        bgcolor: "transparent",
                      },
                    }}
                  >
                    <ChevronRight sx={{ fontSize: "1.25em" }} />
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
          {showPageTop && (
            <Tooltip title="Back to page top">
              <IconButton
                aria-label="Back to page top"
                onClick={scrollPageToTop}
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
                <ArrowUpward sx={{ fontSize: "1.25em" }} />
              </IconButton>
            </Tooltip>
          )}
          <Tooltip title={composeOpen ? "Close question editor" : "Open question editor"}>
            <IconButton
              color="primary"
              aria-label={composeOpen ? "Close question editor" : "Open question editor"}
              aria-pressed={composeOpen}
              onClick={(): void => onComposeToggle(knownPageIds)}
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
        footer={<MobileSheetDismiss onClose={closePageDirectory} />}
        footerOverlay
      >
        <Box
          sx={{
            height: "100%",
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
              pages={pages}
              currentId={current?.id ?? null}
              firstOrdinal={Math.max(1, total - pages.length + 1)}
              hasEarlier={hasEarlierHistory}
              loadingEarlier={loadingEarlier}
              // Directory pagination is page-semantic, not transport-semantic:
              // one answer can span many bounded event batches, none of which
              // adds a visible row here. Fetch through the question boundary so
              // one upward approach always reveals an earlier directory page.
              onReachStart={(): void => void loadPreviousQuestionPage(sessionId)}
              onDismiss={closePageDirectory}
              onSelect={(id): void => {
                navigate(id);
                setOpen(false);
              }}
            />
          </Box>
        </Box>
      </DetentSheet>
    </>
  );
}
