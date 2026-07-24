import {
  ChevronLeft,
  ChevronRight,
  ListAltOutlined,
  Search,
  VerticalAlignBottom,
} from "@mui/icons-material";
import {
  alpha,
  Box,
  Button,
  Divider,
  Drawer,
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
import { useEffect, useMemo, useRef, useState } from "react";
import { derive } from "../derive";
import type { Envelope, Status } from "../protocol";
import { useStoreSelector } from "../store";
import { Transcript } from "../Transcript";
import {
  setExplorePage,
  resolveProjectionAnchor,
  resolveExploreFollowUp,
  useExploreSessionState,
} from "./exploreStore";
import {
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
} {
  const {
    pageId,
    pageParents,
    pendingFollowUp,
    transitionAnchorKey,
  } = useExploreSessionState(sessionId);
  const basePages = useMemo(
    () => deriveQuestionPages(derive(timeline)),
    [timeline],
  );
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
  const currentIndex = Math.max(
    0,
    selectedPageId
      ? pages.findIndex((page) => page.id === selectedPageId)
      : pages.length - 1,
  );
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
  const { pages, current, currentIndex, select } = usePages(sessionId, timeline);
  const [open, setOpen] = useState(false);
  const previous = pages[currentIndex - 1];
  const next = pages[currentIndex + 1];
  const knownPageIds = pages.map((page) => page.id);
  const atTail = pages.length === 0 || currentIndex === pages.length - 1;

  useEffect(() => {
    onTailChange?.(atTail);
  }, [atTail, onTailChange]);

  return (
    <>
      <Paper
        component="nav"
        aria-label="Question pages"
        elevation={0}
        sx={{
          mx: "max(env(safe-area-inset-left), 12px)",
          my: 1,
          minHeight: 56,
          px: 0.75,
          display: "flex",
          alignItems: "center",
          gap: 0.5,
          border: 1,
          borderColor: "divider",
          borderRadius: 2.5,
          bgcolor: (theme) => alpha(theme.palette.background.paper, 0.78),
          backdropFilter: "blur(24px) saturate(180%)",
          WebkitBackdropFilter: "blur(24px) saturate(180%)",
        }}
      >
        <Stack
          direction="row"
          sx={{
            flex: 1,
            minWidth: 0,
            height: 44,
            border: 1,
            borderColor: "divider",
            borderRadius: 1.75,
            overflow: "hidden",
          }}
        >
          <Tooltip title="Previous page">
            <span>
              <IconButton
                aria-label="Previous page"
                disabled={!previous}
                onClick={(): void => previous && select(previous.id)}
                sx={{ width: 44, height: 44, borderRadius: 0 }}
              >
                <ChevronLeft />
              </IconButton>
            </span>
          </Tooltip>
          <Divider orientation="vertical" flexItem />
          <Tooltip title="Next page">
            <span>
              <IconButton
                aria-label="Next page"
                disabled={!next}
                onClick={(): void => next && select(next.id)}
                sx={{ width: 44, height: 44, borderRadius: 0 }}
              >
                <ChevronRight />
              </IconButton>
            </span>
          </Tooltip>
          <Divider orientation="vertical" flexItem />
          <Button
            aria-label="Open question pages"
            onClick={(): void => setOpen(true)}
            startIcon={<ListAltOutlined />}
            sx={{
              minWidth: 0,
              flex: 1,
              px: 1,
              borderRadius: 0,
              textTransform: "none",
              overflow: "hidden",
            }}
          >
            <Typography
              variant="caption"
              noWrap
              sx={{ fontWeight: 750, fontVariantNumeric: "tabular-nums" }}
            >
              {pages.length === 0
                ? "Pages"
                : `${String(currentIndex + 1)} / ${String(pages.length)}`}
            </Typography>
          </Button>
        </Stack>
        <Tooltip title="Return to latest page and type">
          <IconButton
            color="primary"
            aria-label="Return to latest page and type"
            onClick={(): void => {
              const latest = pages.at(-1);
              if (latest) select(latest.id);
              onCompose("new_page", null, knownPageIds);
            }}
            sx={{ width: 44, height: 44 }}
          >
            <VerticalAlignBottom />
          </IconButton>
        </Tooltip>
      </Paper>
      <Drawer
        anchor="bottom"
        open={open}
        onClose={(): void => setOpen(false)}
        PaperProps={{
          sx: {
            height: "min(78dvh, 760px)",
            borderTopLeftRadius: 24,
            borderTopRightRadius: 24,
            overflow: "hidden",
            pb: "env(safe-area-inset-bottom)",
          },
        }}
      >
        <Box
          aria-hidden
          sx={{
            width: 44,
            height: 5,
            borderRadius: 99,
            bgcolor: "text.disabled",
            mx: "auto",
            mt: 1.25,
          }}
        />
        <Stack direction="row" alignItems="baseline" sx={{ px: 2, pt: 1.5 }}>
          <Typography variant="h6" sx={{ fontWeight: 750 }}>Pages</Typography>
          <Typography variant="caption" color="text.secondary" sx={{ ml: 1 }}>
            {String(pages.length)}
          </Typography>
        </Stack>
        <Box sx={{ flex: 1, minHeight: 0 }}>
          <PageList
            pages={pages}
            currentId={current?.id ?? null}
            onSelect={(id): void => {
              select(id);
              setOpen(false);
            }}
          />
        </Box>
        <Button
          startIcon={<VerticalAlignBottom />}
          onClick={(): void => {
            setOpen(false);
            const latest = pages.at(-1);
            if (latest) select(latest.id);
            onCompose("new_page", null, knownPageIds);
          }}
          sx={{ mx: 2, mb: 1, minHeight: 48, textTransform: "none" }}
        >
          Return to latest page and type
        </Button>
      </Drawer>
    </>
  );
}
