import {
  ChevronLeft,
  ChevronRight,
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
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { DetentSheet, MobileSheetDismiss } from "../_shell";
import { derive } from "../derive";
import type { Envelope, Status } from "../protocol";
import {
  loadOlder,
  loadPreviousQuestionPage,
  useStoreSelector,
} from "../store";
import { Transcript } from "../Transcript";
import {
  setExplorePage,
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
    select: (id: string): void => setExplorePage(sessionId, id),
    navigate: (id: string): void => navigateExplorePage(sessionId, id),
  };
}

function PageList({
  pages,
  currentId,
  onSelect,
  dense = false,
}: {
  pages: QuestionPage[];
  currentId: string | null;
  onSelect: (id: string) => void;
  dense?: boolean;
}): React.JSX.Element {
  const [query, setQuery] = useState("");
  const selectedRef = useRef<HTMLDivElement | null>(null);
  const filtered = query.trim()
    ? pages.filter((page) =>
      page.title.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())
    )
    : pages;

  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "center" });
  }, [currentId]);

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
      <List
        dense={dense}
        sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: dense ? 0.75 : 1 }}
      >
        {filtered.map((page, index) => {
          const selected = page.id === currentId;
          return (
            <ListItemButton
              key={page.id}
              ref={selected ? selectedRef : undefined}
              selected={selected}
              onClick={(): void => onSelect(page.id)}
              sx={{
                borderRadius: 1.5,
                mb: 0.5,
                alignItems: "flex-start",
                border: 1,
                borderColor: selected ? "primary.main" : "transparent",
                "&.Mui-selected": {
                  bgcolor: (theme) => alpha(theme.palette.primary.main, 0.09),
                },
              }}
            >
              <Typography
                variant="caption"
                color={selected ? "primary.main" : "text.secondary"}
                sx={{ width: 28, pt: 0.25, fontVariantNumeric: "tabular-nums" }}
              >
                {String(index + 1)}
              </Typography>
              <ListItemText
                primary={page.title}
                secondary={`${String(page.questionCount)} question${
                  page.questionCount === 1 ? "" : "s"
                }`}
                slotProps={{
                  primary: {
                    noWrap: true,
                    fontWeight: selected ? 700 : 500,
                  },
                  secondary: { noWrap: true },
                }}
              />
            </ListItemButton>
          );
        })}
      </List>
    </Stack>
  );
}

export function ExploreTranscript(
  props: ExploreTranscriptProps,
): React.JSX.Element {
  const { pages, current, currentIndex, select } = usePages(
    props.sessionId,
    props.timeline,
  );
  const visibleItemKeys = useMemo(
    () => new Set(current?.itemKeys ?? []),
    [current?.itemKeys],
  );
  const atTail = currentIndex === pages.length - 1;
  const rootRef = useRef<HTMLDivElement>(null);
  const { pageStartId } = useExploreSessionState(props.sessionId);

  useLayoutEffect(() => {
    if (!current || pageStartId !== current.id) return;
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
              onSelect={select}
            />
          </Box>
        </>
      )}
      <Stack sx={{ flex: 1, minWidth: 0, minHeight: 0 }}>
        {current && (
          <Box
            sx={{
              minHeight: 40,
              px: { xs: 2, md: 2.5 },
              display: "flex",
              alignItems: "center",
              gap: 1,
              borderBottom: 1,
              borderColor: "divider",
              flexShrink: 0,
            }}
          >
            <Typography
              variant="caption"
              color="primary.main"
              sx={{ fontWeight: 750, fontVariantNumeric: "tabular-nums" }}
            >
              {String(currentIndex + 1)} / {String(pages.length)}
            </Typography>
            <Typography variant="body2" noWrap sx={{ fontWeight: 650 }}>
              {current.title}
            </Typography>
          </Box>
        )}
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
          liveTail={atTail}
          shortContentAtTop
        />
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
  onCompose,
  onTailChange,
}: {
  sessionId: string;
  onCompose: (
    intent: "follow_up" | "new_page",
    targetPageId: string | null,
    knownPageIds: string[],
  ) => void;
  onTailChange?: ((atTail: boolean) => void) | undefined;
}): React.JSX.Element {
  const timeline = useStoreSelector((snapshot) =>
    snapshot.timelines.get(sessionId) ?? EMPTY_TIMELINE
  );
  const pagination = useStoreSelector((snapshot) =>
    snapshot.pagination.get(sessionId)
  );
  const { pages, current, currentIndex, navigate } = usePages(sessionId, timeline);
  const [pageIndex, setPageIndex] = useState<{ total: number; exact: boolean } | null>(
    null,
  );
  const [open, setOpen] = useState(false);
  const [pendingPrevious, setPendingPrevious] = useState<{
    anchorItemKey: string;
    requestedBeforeSeq: number | null;
    requestComplete: boolean;
  } | null>(null);
  const previous = current
    ? completePageBeforeItem(pages, current.itemKeys.at(-1) ?? "")
    : undefined;
  const next = pages[currentIndex + 1];
  const knownPageIds = pages.map((page) => page.id);
  const atTail = pages.length === 0 || currentIndex === pages.length - 1;
  const hasEarlierHistory = pagination?.reachedStart === false;
  const loadingEarlier = pagination?.loadingOlder === true;
  const loadingPrevious = loadingEarlier || pendingPrevious !== null;
  const onlyCompletePage = pages.length <= 1 && !hasEarlierHistory;
  const total = Math.max(pages.length, pageIndex?.total ?? 0);
  const currentOrdinal = Math.max(
    1,
    total - Math.max(0, pages.length - 1 - currentIndex),
  );

  useEffect(() => {
    const controller = new AbortController();
    void fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/question-pages`,
      { signal: controller.signal },
    )
      .then((response) => {
        if (!response.ok) throw new Error(`question pages: ${String(response.status)}`);
        return response.json() as Promise<{ total: number; exact: boolean }>;
      })
      .then(setPageIndex)
      .catch((error: unknown) => {
        if (!(error instanceof DOMException && error.name === "AbortError")) {
          console.warn("Failed to load exact question page count", error);
        }
      });
    return () => controller.abort();
  }, [sessionId, pages.at(-1)?.id]);

  const goPrevious = (): void => {
    if (previous) {
      navigate(previous.id);
      return;
    }
    if (!hasEarlierHistory || loadingPrevious || !current) return;
    // One answer can span several bounded HTTP history pages. Keep loading
    // under this single user action until the preceding question boundary is
    // found, rather than making the reader tap once per transport page.
    const anchorItemKey = current.itemKeys.at(-1);
    if (!anchorItemKey) return;
    setPendingPrevious({
      anchorItemKey,
      requestedBeforeSeq: null,
      requestComplete: true,
    });
  };

  useEffect(() => {
    onTailChange?.(atTail);
  }, [atTail, onTailChange]);

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
          height: 52,
          mx: "max(env(safe-area-inset-left), 12px)",
          mr: "max(env(safe-area-inset-right), 12px)",
          mt: 1.5,
          mb: 0.75,
          px: 0.5,
          display: "grid",
          gridTemplateColumns: "1fr auto 1fr",
          alignItems: "center",
          borderColor: "divider",
          borderRadius: 2,
          overflow: "hidden",
          bgcolor: "transparent",
        }}
      >
        <Stack direction="row" sx={{ justifySelf: "start" }}>
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
                      width: 40,
                      height: 40,
                      m: 0.25,
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
                      : <ChevronLeft sx={{ fontSize: "1.25rem" }} />}
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
                      width: 40,
                      height: 40,
                      m: 0.25,
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
                    <ChevronRight sx={{ fontSize: "1.25rem" }} />
                  </IconButton>
                </span>
              </Tooltip>
            </>
          )}
        </Stack>
        <Button
          aria-label="Open question pages"
          onClick={(): void => setOpen(true)}
          startIcon={<ListAltOutlined sx={{ fontSize: "1.25rem" }} />}
          sx={{
            minWidth: 92,
            height: 40,
            px: 1.25,
            border: 1,
            borderColor: "divider",
            borderRadius: 20,
            bgcolor: "action.hover",
            textTransform: "none",
            color: "text.primary",
            "&:active": { bgcolor: "action.selected" },
          }}
        >
          <Typography
            variant="caption"
            noWrap
            sx={{ fontWeight: 750, fontVariantNumeric: "tabular-nums" }}
          >
            {pages.length === 0
              ? "Pages"
              : `${String(currentOrdinal)} / ${String(total)}${
                pageIndex?.exact === false ? "+" : ""
              }`}
          </Typography>
        </Button>
        <Tooltip title="Latest page and type">
          <IconButton
            color="primary"
            aria-label="Latest page and type"
            onClick={(): void => {
              const latest = pages.at(-1);
              if (latest) navigate(latest.id);
              onCompose("new_page", null, knownPageIds);
            }}
            sx={{
              width: 40,
              height: 40,
              m: 0.25,
              justifySelf: "end",
              border: 1,
              borderColor: "divider",
              borderRadius: "50%",
              bgcolor: "action.hover",
              "&:active": { bgcolor: "action.selected" },
            }}
          >
            <EditOutlined sx={{ fontSize: "1.25rem" }} />
          </IconButton>
        </Tooltip>
      </Paper>
      <DetentSheet
        open={open}
        onClose={(): void => setOpen(false)}
        ariaLabel="Question pages"
        frosted
        footer={<MobileSheetDismiss onClose={(): void => setOpen(false)} />}
        footerOverlay
      >
        <Box
          sx={{
            height: "min(72dvh, 700px)",
            display: "flex",
            flexDirection: "column",
            minHeight: 0,
          }}
        >
          <Stack direction="row" alignItems="baseline" sx={{ px: 2, pt: 1.5 }}>
            <Typography variant="h6" sx={{ fontWeight: 750 }}>Pages</Typography>
            <Typography variant="caption" color="text.secondary" sx={{ ml: 1 }}>
              {`${String(pages.length)}${hasEarlierHistory ? "+" : ""}`}
            </Typography>
          </Stack>
          {hasEarlierHistory && (
            <Button
              disabled={loadingEarlier}
              startIcon={loadingEarlier
                ? <CircularProgress size={16} />
                : <ChevronLeft />}
              onClick={(): void => void loadOlder(sessionId)}
              sx={{ mx: 2, mt: 1, minHeight: 40, textTransform: "none" }}
            >
              {loadingEarlier ? "Loading…" : "Load earlier questions"}
            </Button>
          )}
          <Box sx={{ flex: 1, minHeight: 0 }}>
            <PageList
              pages={pages}
              currentId={current?.id ?? null}
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
