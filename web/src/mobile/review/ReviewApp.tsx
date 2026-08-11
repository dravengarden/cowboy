import {
  AccountTreeOutlined,
  ArrowBack,
  ArrowForward,
  CheckCircle,
  CheckCircleOutline,
  ChevronLeft,
  ChevronRight,
  Close,
  DifferenceOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  FormatListBulleted,
  KeyboardArrowDown,
  KeyboardArrowUp,
  Refresh,
  VisibilityOutlined,
  WrapText,
} from "@mui/icons-material";
import {
  Badge,
  Box,
  Button,
  Chip,
  CircularProgress,
  Divider,
  IconButton,
  ListItemButton,
  Popover,
  Stack,
  Toolbar,
  Typography,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  setActiveSessionId,
  useActiveWorkspaceBinding,
  useControlPlaneSessionActivity,
} from "../../controlPlane";
import { importantHaptic, navigationHaptic } from "../../haptic";
import { Markdown } from "../../Markdown";
import { Sheet } from "../../Sheet";
import {
  mutateMobileReview,
  openSession,
  useMobileReviewState,
  useStoreSelector,
} from "../../store";
import type { SessionMeta } from "../../protocol";
import { sessionProjectLabel } from "../../sessionProject";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";
import { newUuid } from "../../uuid";
import {
  closeCodeBuffer,
  CodeApiError,
  type CodeHover,
  type CodeLocation,
  type CodeNavigationKind,
  fetchCodeChanges,
  fetchCodeDiffPage,
  fetchCodeFile,
  fetchCodeFilePage,
  fetchCodeHover,
  fetchCodeLanguage,
  fetchCodeManifest,
  fetchCodeNavigation,
  isTransientCodeApiStatus,
  openCodeBuffer,
  shouldCloseUnavailableSource,
} from "./codeApi";
import { invalidateDiffCache, loadCodeDiff } from "./diffCache";
import { diffHunkLines, reviewEntryKey } from "./diffNavigationModel";
import { type ReviewProgress, revisionMatches } from "./reviewProgress";
import { ReviewRepository } from "./ReviewRepository";
import type { CodeInspectCandidate, CodeRevealRange } from "./CodeViewer";
import { ReviewDrawerShell } from "./ReviewDrawerShell";
import { ReviewFileTree } from "./ReviewFileTree";
import { ReviewOutline } from "./ReviewOutline";
import { ReviewSettings } from "./ReviewSettings";
import { isMarkdownReviewPath } from "./reviewMarkdown";
import type { ReviewMode } from "./reviewMode";
import { updateReviewSettings, useReviewSettings } from "./reviewSettings";
import { presentHoverBlock } from "./symbolPresentation";
import type { CodeDiffScope } from "./codeApi";
import type { CodeLanguage } from "./codeApi";
import type { GitReviewEntry } from "./gitReviewModel";
import { groupGitChanges, reviewQueue } from "./gitReviewModel";
import { ReviewTabStrip } from "./ReviewTabStrip";
import {
  adjacentReviewTabAfterClose,
  closeAllReviewTabs,
  closeOtherReviewTabs,
  closeReviewTab,
  openReviewTab,
  reorderReviewTabs,
  retainChangedDiffTabs,
  type ReviewTab,
  reviewTabKey,
  toggleReviewTabPin,
} from "./reviewTabs";

const CodeViewer = lazy(() => import("./CodeViewer"));

function sessionProject(session: SessionMeta): string {
  const stableProject = sessionProjectLabel(session);
  if (stableProject) return stableProject;
  const normalized = session.cwd.replace(/\/+$/, "");
  return normalized.split("/").at(-1) || session.cwd;
}

function sessionStatusColor(status: SessionMeta["status"]): string {
  switch (status) {
    case "running":
    case "busy":
      return "success.main";
    case "starting":
      return "warning.main";
    case "crashed":
      return "error.main";
    default:
      return "text.disabled";
  }
}

type ReviewTarget =
  | { kind: "changes" }
  | {
    kind: "diff";
    path: string;
    scope: CodeDiffScope;
    queue: GitReviewEntry[];
  }
  | {
    kind: "source";
    path: string;
    revealLine?: number;
    revealRange?: CodeRevealRange;
    revealRequestId?: number;
  };

type SymbolPoint = { row: number; column: number };
type CodeNavigationEntry = Extract<ReviewTarget, { kind: "source" }> & {
  symbol?: SymbolPoint;
  navigationRange?: Omit<CodeRevealRange, "id">;
};
type SymbolRestoreRequest = SymbolPoint & { path: string; id: number };
type TabCloseRequest =
  | { kind: "one"; key: string; anchor: HTMLElement }
  | { kind: "others"; key: string; anchor: HTMLElement }
  | { kind: "all"; anchor: HTMLElement };

function DocumentView({
  sessionId,
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
}: {
  sessionId: string;
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
}): React.JSX.Element {
  const surface = useSurfaceProfile();
  const settings = useReviewSettings();
  const [text, setText] = useState("");
  const [truncated, setTruncated] = useState(false);
  const [limited, setLimited] = useState(false);
  const [nextCursor, setNextCursor] = useState<string>();
  const [revision, setRevision] = useState<string>();
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadMoreError, setLoadMoreError] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
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
  const [inspectCandidates, setInspectCandidates] = useState<
    CodeInspectCandidate[]
  >([]);
  const [inspectCandidatesExpanded, setInspectCandidatesExpanded] = useState(
    false,
  );
  const [inspectAnchor, setInspectAnchor] = useState<
    { top: number; left: number } | undefined
  >();
  const [navigationLoading, setNavigationLoading] = useState(false);
  const hoverController = useRef<AbortController | undefined>(undefined);
  const fileRetry = useRef<{ key: string; count: number }>({
    key: "",
    count: 0,
  });
  const positionTimer = useRef<number | undefined>(undefined);
  const pendingPosition = useRef<number | undefined>(undefined);
  const hunks = target.kind === "diff" ? diffHunkLines(text) : [];

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
    if (target.kind === "diff" && target.scope !== "unstaged") return;
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
        const value = await fetchCodeHover(
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
        onBufferUnavailable();
      }
    }).finally(() => {
      if (!controller.signal.aborted) setHoverLoading(false);
    });
  }, [onBufferUnavailable, sessionId, target]);

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

  const navigate = useCallback((kind: CodeNavigationKind): void => {
    if (
      !inspectTarget ||
      (target.kind === "diff" && target.scope !== "unstaged")
    ) return;
    hoverController.current?.abort();
    const controller = new AbortController();
    hoverController.current = controller;
    setNavigation([]);
    setNavigationLoading(true);
    void fetchCodeNavigation(
      sessionId,
      target.path,
      inspectTarget.row,
      inspectTarget.column,
      kind,
      controller.signal,
    ).then((result) => {
      if (controller.signal.aborted) return;
      const onlyLocation = result.locations.length === 1
        ? result.locations[0]
        : undefined;
      if (onlyLocation) {
        setHoverOpen(false);
        onNavigate(onlyLocation, inspectTarget);
      } else {
        setNavigation(result.locations);
      }
    }).catch(() => {
      if (!controller.signal.aborted) setNavigation([]);
    }).finally(() => {
      if (!controller.signal.aborted) setNavigationLoading(false);
    });
  }, [inspectTarget, onNavigate, sessionId, target]);

  useEffect(() => {
    const controller = new AbortController();
    const diffTarget = target.kind === "diff" ? target : undefined;
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
    const request = diffTarget
      ? loadCodeDiff(
        sessionId,
        diffTarget.path,
        settings.contextLines,
        settings.showWhitespaceChanges,
        diffTarget.scope,
        controller.signal,
      )
      : fetchCodeFile(sessionId, target.path, controller.signal);
    void request.then((result) => {
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
          entry.scope === diffTarget.scope
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
            ).catch(() => undefined);
          }
        }
      }
    }).catch((reason) => {
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
    onRevision,
    reloadKey,
  ]);

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
              ? "Cowboy can preview UTF-8 text files, but not compiled programs or other binary data."
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
  const collapsedCandidates = selectedCandidate
    ? [
      selectedCandidate,
      ...inspectCandidates.filter((candidate) =>
        candidate !== selectedCandidate
      ).slice(0, 1),
    ]
    : [];
  const hiddenCandidateCount = Math.max(
    0,
    inspectCandidates.length - collapsedCandidates.length,
  );
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
        <Stack
          direction="row"
          useFlexGap
          gap={0.75}
          alignItems="flex-start"
        >
          <Stack
            direction="row"
            useFlexGap
            flexWrap={inspectCandidatesExpanded ? "wrap" : "nowrap"}
            gap={0.75}
            sx={{
              flex: 1,
              minWidth: 0,
              ...(inspectCandidatesExpanded
                ? {
                  maxHeight: 120,
                  overflowY: "auto",
                  overscrollBehaviorY: "contain",
                  touchAction: "pan-y",
                  WebkitOverflowScrolling: "touch",
                }
                : { overflow: "hidden" }),
            }}
          >
            {(inspectCandidatesExpanded
              ? inspectCandidates
              : collapsedCandidates).map(
                candidateChip,
              )}
          </Stack>
          {(hiddenCandidateCount > 0 || inspectCandidatesExpanded) && (
            <Button
              size="small"
              variant="text"
              aria-expanded={inspectCandidatesExpanded}
              onClick={() => {
                navigationHaptic();
                setInspectCandidatesExpanded((expanded) => !expanded);
              }}
              endIcon={inspectCandidatesExpanded
                ? <KeyboardArrowUp />
                : <KeyboardArrowDown />}
              sx={{
                minWidth: 0,
                height: 32,
                px: 1,
                flex: "0 0 auto",
                textTransform: "none",
              }}
            >
              {inspectCandidatesExpanded
                ? "Collapse"
                : `+${hiddenCandidateCount}`}
            </Button>
          )}
        </Stack>
      </Box>
    )
    : null;
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
      {!hoverLoading && !hoverError && (
        <Stack direction="row" useFlexGap flexWrap="wrap" gap={1}>
          <Button
            size="small"
            onClick={() =>
              navigate("definition")}
          >
            Definition
          </Button>
          <Button
            size="small"
            onClick={() =>
              navigate("declaration")}
          >
            Declaration
          </Button>
          <Button
            size="small"
            onClick={() =>
              navigate("typeDefinition")}
          >
            Type
          </Button>
          <Button
            size="small"
            onClick={() =>
              navigate("implementation")}
          >
            Implementations
          </Button>
          <Button size="small" onClick={() => navigate("references")}>
            References
          </Button>
        </Stack>
      )}
      {navigationLoading && <CircularProgress size={20} />}
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
    <Stack sx={{ flex: 1, minHeight: 0 }}>
      {(truncated || counts || nextCursor || loadMoreError) && (
        <Stack
          direction="row"
          spacing={1}
          alignItems="center"
          sx={{ px: 1.5, py: 0.75, borderBottom: 1, borderColor: "divider" }}
        >
          {counts && (
            <>
              <Typography variant="caption" color="success.main">
                +{counts.added}
              </Typography>
              <Typography variant="caption" color="error.main">
                −{counts.removed}
              </Typography>
            </>
          )}
          {hunks.length > 0 && (
            <>
              <Box sx={{ flex: 1 }} />
              <IconButton
                size="small"
                aria-label="Previous hunk"
                disabled={hunkIndex <= 0}
                onClick={() => setHunkIndex((value) => value - 1)}
              >
                <KeyboardArrowUp fontSize="small" />
              </IconButton>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ minWidth: 36, textAlign: "center" }}
              >
                {hunkIndex + 1}/{hunks.length}
              </Typography>
              <IconButton
                size="small"
                aria-label="Next hunk"
                disabled={hunkIndex >= hunks.length - 1}
                onClick={() => setHunkIndex((value) => value + 1)}
              >
                <KeyboardArrowDown fontSize="small" />
              </IconButton>
            </>
          )}
          {nextCursor && (
            <Button
              size="small"
              variant="outlined"
              disabled={loadingMore}
              onClick={loadMore}
              startIcon={loadingMore
                ? <CircularProgress size={12} />
                : undefined}
            >
              {loadMoreError ? "Retry" : "Load more"}
            </Button>
          )}
          {limited && !nextCursor && (
            <Chip
              size="small"
              color="warning"
              label={target.kind === "diff"
                ? "Preview limited to 16 MB"
                : "Preview limited to 32 MB"}
            />
          )}
        </Stack>
      )}
      <Box
        sx={{
          flex: 1,
          minHeight: 0,
          overflow: markdownPreview ? "auto" : "hidden",
        }}
      >
        {markdownPreview
          ? (
            <Box
              component="article"
              data-markdown-review-preview
              sx={{
                width: "100%",
                maxWidth: 880,
                mx: "auto",
                px: { xs: 2.25, sm: 4 },
                py: 2.5,
                lineHeight: 1.6,
                overflowWrap: "anywhere",
                "& h1": {
                  fontSize: "1.75rem !important",
                  pb: 0.5,
                  borderBottom: 1,
                  borderColor: "divider",
                },
                "& h2": {
                  fontSize: "1.4rem !important",
                  pb: 0.35,
                  borderBottom: 1,
                  borderColor: "divider",
                },
                "& h3": { fontSize: "1.18rem !important" },
                "& hr": {
                  border: 0,
                  borderTop: 1,
                  borderColor: "divider",
                  my: 2,
                },
              }}
            >
              <Markdown text={text} touchWrap />
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
                text={text}
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
                  ? languageData
                  : undefined}
                diagnostics={settings.diagnostics}
                inlayHints={settings.inlayHints}
                semanticHighlighting={settings.semanticHighlighting}
                onInspect={target.kind === "source" ||
                    (target.kind === "diff" && target.scope === "unstaged")
                  ? inspectCandidatesOrPoint
                  : undefined}
                onVisibleLine={target.kind === "source"
                  ? persistVisibleLine
                  : undefined}
              />
            </Suspense>
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
    </Stack>
  );
}

export function ReviewApp({
  active,
  onDrawerOpenChange,
}: {
  active: boolean;
  onDrawerOpenChange: (open: boolean) => void;
}): React.JSX.Element {
  const workspace = useActiveWorkspaceBinding();
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const controlPlaneActivity = useControlPlaneSessionActivity(
    workspace?.sessionId,
  );
  const settings = useReviewSettings();
  const [mode, setMode] = useState<ReviewMode>("git");
  const [markdownPreview, setMarkdownPreview] = useState(true);
  const [sourceTarget, setSourceTarget] = useState<
    Extract<ReviewTarget, { kind: "source" }> | undefined
  >();
  const [diffTarget, setDiffTarget] = useState<
    Extract<ReviewTarget, { kind: "diff" }> | undefined
  >();
  const target: ReviewTarget = mode === "files"
    ? sourceTarget ?? { kind: "changes" }
    : diffTarget ?? { kind: "changes" };
  const leasedPath = target.kind === "changes" ? undefined : target.path;
  const [closeRequest, setCloseRequest] = useState(0);
  const [toggleDrawerRequest, setToggleDrawerRequest] = useState(0);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [sessionSwitcherOpen, setSessionSwitcherOpen] = useState(false);
  const sessionListRef = useRef<HTMLDivElement>(null);
  const [reviewProgress, setReviewProgress] = useState<ReviewProgress>({});
  const [currentRevision, setCurrentRevision] = useState<string>();
  const [dataRevision, setDataRevision] = useState(0);
  const [changeCount, setChangeCount] = useState(0);
  const [repositoryContext, setRepositoryContext] = useState<{
    project: string;
    branch: string | undefined;
    worktree: string | undefined;
    head: string | undefined;
  }>();
  const [language, setLanguage] = useState<
    import("./codeApi").CodeLanguageCapabilities
  >();
  const [languageData, setLanguageData] = useState<CodeLanguage>();
  const [tabs, setTabs] = useState<ReviewTab[]>([]);
  const [tabCloseRequest, setTabCloseRequest] = useState<TabCloseRequest>();
  const [gitQueue, setGitQueue] = useState<GitReviewEntry[]>([]);
  const [navigationHistory, setNavigationHistory] = useState<
    CodeNavigationEntry[]
  >([]);
  const [navigationForwardHistory, setNavigationForwardHistory] = useState<
    CodeNavigationEntry[]
  >([]);
  const [symbolRestore, setSymbolRestore] = useState<SymbolRestoreRequest>();
  const [closeSymbolRequest, setCloseSymbolRequest] = useState(0);
  const currentSymbol = useRef<SymbolPoint | undefined>(undefined);
  const currentVisibleLine = useRef<number | undefined>(undefined);
  const currentNavigationFrame = useRef<CodeNavigationEntry | undefined>(
    undefined,
  );
  const symbolRestoreId = useRef(0);
  const revealRangeId = useRef(0);
  const [outlineOpen, setOutlineOpen] = useState(false);
  const syncedReview = useMobileReviewState(workspace?.sessionId);
  const syncedReviewRef = useRef(syncedReview);
  syncedReviewRef.current = syncedReview;
  const [manifestRefreshRequest, setManifestRefreshRequest] = useState(0);
  const [bufferRecoveryRequest, setBufferRecoveryRequest] = useState(0);
  const requestBufferRecovery = useCallback((): void => {
    setBufferRecoveryRequest((request) => request + 1);
  }, []);
  const manifestRevision = useRef<string | undefined>(undefined);
  const adoptManifestRevision = useCallback((revision: string): void => {
    manifestRevision.current = revision;
  }, []);
  const handleDrawerOpenChange = useCallback((open: boolean): void => {
    setDrawerOpen(open);
    onDrawerOpenChange(open);
  }, [onDrawerOpenChange]);

  useEffect(() => {
    if (
      !workspace?.sessionId ||
      !leasedPath
    ) {
      setLanguageData(undefined);
      return undefined;
    }
    const sessionId = workspace.sessionId;
    const path = leasedPath;
    const leaseId = newUuid();
    let released = false;
    let opened = false;
    let openAttempt = 0;
    const retryTimers: number[] = [];
    const release = (): void => {
      if (!opened) return;
      opened = false;
      void closeCodeBuffer(sessionId, path, leaseId).catch(() => undefined);
    };
    const loadLanguage = (): void => {
      void fetchCodeLanguage(sessionId, path)
        .then((value) => {
          if (!released) setLanguageData(value);
        })
        // File rendering remains useful while a language server is cold or
        // unavailable.
        .catch(() => undefined);
    };
    const acquire = (): void => {
      const attempt = openAttempt++;
      void openCodeBuffer(sessionId, path, leaseId)
        .then(() => {
          opened = true;
          if (released) {
            release();
            return;
          }
          loadLanguage();
          // Zed starts an LSP only after the first buffer registration. Keep
          // the source visible immediately, then revalidate after typical warm
          // and cold language-server startup windows.
          retryTimers.push(globalThis.setTimeout(loadLanguage, 8_000));
          retryTimers.push(globalThis.setTimeout(loadLanguage, 30_000));
        })
        .catch((reason) => {
          const status = reason instanceof CodeApiError
            ? reason.status
            : undefined;
          if (
            released || attempt >= 3 ||
            !isTransientCodeApiStatus(status)
          ) return;
          const delay = [1_000, 4_000, 12_000][attempt] ?? 12_000;
          retryTimers.push(globalThis.setTimeout(acquire, delay));
        });
    };
    acquire();
    return () => {
      released = true;
      retryTimers.forEach((timer) => globalThis.clearTimeout(timer));
      setLanguageData(undefined);
      release();
    };
  }, [
    bufferRecoveryRequest,
    dataRevision,
    leasedPath,
    workspace?.sessionId,
  ]);

  useEffect(() => {
    setTabCloseRequest(undefined);
    manifestRevision.current = undefined;
    setDataRevision(0);
    setChangeCount(0);
    setRepositoryContext(undefined);
    setGitQueue([]);
    setLanguage(undefined);
  }, [workspace?.sessionId]);

  useEffect(() => {
    if (!active || !workspace?.sessionId) return undefined;
    // ACP streams can emit many updates for one tool call. Coalesce them into
    // one filesystem revalidation after the burst instead of repeatedly
    // running git status while text is still streaming.
    const timer = globalThis.setTimeout(
      () => setManifestRefreshRequest((value) => value + 1),
      600,
    );
    return () => globalThis.clearTimeout(timer);
  }, [active, controlPlaneActivity, workspace?.sessionId]);

  useEffect(() => {
    if (!active || !workspace?.sessionId) return undefined;
    let controller: AbortController | undefined;
    const refreshManifest = (): void => {
      controller?.abort();
      controller = new AbortController();
      void fetchCodeManifest(workspace.sessionId, controller.signal)
        .then((manifest) => {
          setChangeCount(manifest.changeCount);
          setRepositoryContext({
            project: manifest.project,
            branch: manifest.branch,
            worktree: manifest.worktree,
            head: manifest.head,
          });
          setLanguage(manifest.language);
          const previous = manifestRevision.current;
          manifestRevision.current = manifest.revision;
          if (previous && previous !== manifest.revision) {
            invalidateDiffCache(workspace.sessionId);
            setDataRevision((value) => value + 1);
          }
          return fetchCodeChanges(workspace.sessionId, controller?.signal);
        })
        .then((changes) => {
          setGitQueue(reviewQueue(groupGitChanges(changes.changes)));
        })
        // The ordinary tree/changes error surfaces remain authoritative.
        .catch(() => undefined);
    };
    refreshManifest();
    // Zed/control-plane activity handles the ordinary fast path. Retain a low
    // frequency check for files changed by shells, editors, or Git outside
    // Cowboy, and revalidate immediately when Mobile returns to the foreground.
    const timer = globalThis.setInterval(refreshManifest, 30_000);
    const refreshVisible = (): void => {
      if (document.visibilityState === "visible") refreshManifest();
    };
    document.addEventListener("visibilitychange", refreshVisible);
    globalThis.addEventListener("online", refreshManifest);
    return () => {
      globalThis.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshVisible);
      globalThis.removeEventListener("online", refreshManifest);
      controller?.abort();
    };
  }, [active, manifestRefreshRequest, workspace?.sessionId]);

  useEffect(() => {
    setSourceTarget(undefined);
    setNavigationHistory([]);
    setNavigationForwardHistory([]);
    setSymbolRestore(undefined);
    currentSymbol.current = undefined;
    currentVisibleLine.current = undefined;
    setDiffTarget(undefined);
    setCurrentRevision(undefined);
  }, [workspace?.sessionId]);

  useEffect(() => {
    setMode(syncedReview.mode);
  }, [syncedReview.mode, workspace?.sessionId]);

  useEffect(() => {
    setTabs(syncedReview.tabs.map((tab) => ({ kind: "source", ...tab })));
  }, [syncedReview.tabs, workspace?.sessionId]);

  useEffect(() => {
    setReviewProgress({ ...syncedReview.progress });
  }, [syncedReview.progress, workspace?.sessionId]);

  useEffect(() => {
    const path = syncedReview.active;
    if (!path) {
      setSourceTarget(undefined);
      return;
    }
    setMarkdownPreview(isMarkdownReviewPath(path));
    const line = syncedReviewRef.current.positions?.[path]?.line;
    setSourceTarget((current) =>
      current?.path === path ? current : {
        kind: "source",
        path,
        ...(line === undefined ? {} : {
          revealLine: line,
          revealRequestId: ++revealRangeId.current,
        }),
      }
    );
  }, [syncedReview.active, workspace?.sessionId]);

  const openSource = (
    path: string,
    revealLine?: number,
    preserveNavigation = false,
    revealRange?: Omit<CodeRevealRange, "id">,
  ): void => {
    const revealRequestId = revealLine !== undefined || revealRange
      ? ++revealRangeId.current
      : undefined;
    currentVisibleLine.current = revealLine;
    if (!preserveNavigation) {
      currentNavigationFrame.current = {
        kind: "source",
        path,
        ...(revealLine === undefined ? {} : { revealLine }),
      };
    }
    setCurrentRevision(undefined);
    setMode("files");
    if (workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "open", { path });
    }
    setMarkdownPreview(isMarkdownReviewPath(path));
    setSourceTarget({
      kind: "source",
      path,
      ...(revealLine === undefined ? {} : { revealLine }),
      ...(revealRequestId === undefined ? {} : { revealRequestId }),
      ...(revealRange
        ? { revealRange: { ...revealRange, id: revealRequestId! } }
        : {}),
    });
    setTabs((current) =>
      openReviewTab(current, { kind: "source", path, pinned: false })
    );
    if (!preserveNavigation) {
      setNavigationHistory([]);
      setNavigationForwardHistory([]);
      setSymbolRestore(undefined);
      currentSymbol.current = undefined;
    }
    setCloseRequest((value) => value + 1);
  };
  const restoreNavigationEntry = (entry: CodeNavigationEntry): void => {
    currentNavigationFrame.current = entry;
    currentSymbol.current = undefined;
    currentVisibleLine.current = entry.revealLine;
    setCloseSymbolRequest((value) => value + 1);
    if (entry.symbol) {
      setSymbolRestore({
        path: entry.path,
        ...entry.symbol,
        id: ++symbolRestoreId.current,
      });
    } else {
      setSymbolRestore(undefined);
    }
    const row = Math.max(0, (entry.revealLine ?? 1) - 1);
    openSource(
      entry.path,
      entry.revealLine,
      true,
      entry.navigationRange ?? {
        start: entry.symbol ?? { row, column: 0 },
        end: entry.symbol
          ? { row: entry.symbol.row, column: entry.symbol.column + 1 }
          : { row, column: Number.MAX_SAFE_INTEGER },
      },
    );
  };
  const currentNavigationEntry = (): CodeNavigationEntry | undefined => {
    if (target.kind !== "source") return undefined;
    const frame = currentNavigationFrame.current;
    if (frame?.path === target.path) return frame;
    return {
      kind: "source",
      path: target.path,
      ...(currentVisibleLine.current === undefined
        ? {}
        : { revealLine: currentVisibleLine.current }),
    };
  };
  const navigateBack = (): void => {
    const previous = navigationHistory.at(-1);
    if (!previous) return;
    navigationHaptic();
    const current = currentNavigationEntry();
    if (current) {
      setNavigationForwardHistory((history) =>
        [...history, current].slice(-32)
      );
    }
    setNavigationHistory((history) => history.slice(0, -1));
    restoreNavigationEntry(previous);
  };
  const navigateForward = (): void => {
    const next = navigationForwardHistory.at(-1);
    if (!next) return;
    navigationHaptic();
    const current = currentNavigationEntry();
    if (current) {
      setNavigationHistory((history) => [...history, current].slice(-32));
    }
    setNavigationForwardHistory((history) => history.slice(0, -1));
    restoreNavigationEntry(next);
  };
  const openDiff = (
    entry: GitReviewEntry,
    queue: GitReviewEntry[],
  ): void => {
    setCurrentRevision(undefined);
    setMode("git");
    if (workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "setMode", { mode: "git" });
    }
    setDiffTarget({
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      queue,
    });
    setCloseRequest((value) => value + 1);
  };
  const activateTab = (tab: ReviewTab): void => {
    if (tab.kind === "source") {
      openSource(tab.path);
      return;
    }
    const entry = gitQueue.find((candidate) =>
      candidate.change.path === tab.path && candidate.scope === tab.scope
    );
    if (entry) openDiff(entry, gitQueue);
  };
  const activeTabKey = target.kind === "source"
    ? reviewTabKey({ ...target, pinned: false })
    : target.kind === "diff"
    ? reviewTabKey({ ...target, pinned: false })
    : undefined;
  const gitTabs: ReviewTab[] = gitQueue.map((entry): ReviewTab => {
    const key = reviewTabKey({
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      pinned: false,
    });
    return {
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      pinned: tabs.find((tab) => reviewTabKey(tab) === key)?.pinned ?? false,
    };
  }).sort((left, right) => Number(right.pinned) - Number(left.pinned));
  const modeTabs = mode === "files"
    ? tabs.filter((tab) => tab.kind === "source")
    : gitTabs;
  const activateOrCollapseTab = (tab: ReviewTab): void => {
    if (reviewTabKey(tab) === activeTabKey) {
      navigationHaptic();
      if (tab.kind === "source") setSourceTarget(undefined);
      else setDiffTarget(undefined);
      return;
    }
    activateTab(tab);
  };
  useEffect(() => {
    const changedKeys = new Set(
      gitQueue.map((entry) =>
        reviewTabKey({
          kind: "diff",
          path: entry.change.path,
          scope: entry.scope,
          pinned: false,
        })
      ),
    );
    setTabs((current) => retainChangedDiffTabs(current, changedKeys));
  }, [gitQueue]);
  const closeTab = (key: string): void => {
    const closing = tabs.find((tab) => reviewTabKey(tab) === key);
    const fallback = adjacentReviewTabAfterClose(tabs, key);
    if (closing?.kind === "source" && workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "close", { path: closing.path });
    }
    const next = closeReviewTab(tabs, key);
    setTabs(next);
    if (activeTabKey !== key) return;
    if (fallback) activateTab(fallback);
    else if (mode === "files") setSourceTarget(undefined);
    else setDiffTarget(undefined);
  };
  const closeOtherTabs = (key: string): void => {
    if (workspace?.sessionId) {
      for (const tab of modeTabs) {
        if (
          tab.kind === "source" &&
          reviewTabKey(tab) !== key &&
          !tab.pinned
        ) {
          mutateMobileReview(workspace.sessionId, "close", { path: tab.path });
        }
      }
    }
    const keep = modeTabs.find((tab) => reviewTabKey(tab) === key);
    setTabs((current) => {
      const currentModeTabs = current.filter((tab) => tab.kind === "source");
      const otherModeTabs = current.filter((tab) => tab.kind !== "source");
      return [
        ...otherModeTabs,
        ...closeOtherReviewTabs(currentModeTabs, key),
      ];
    });
    if (keep) activateTab(keep);
  };
  const closeAllTabs = (): void => {
    if (workspace?.sessionId) {
      for (const tab of modeTabs) {
        if (tab.kind === "source") {
          mutateMobileReview(workspace.sessionId, "close", { path: tab.path });
        }
      }
    }
    setTabs((current) => closeAllReviewTabs(current, "source"));
    setSourceTarget(undefined);
    setNavigationHistory([]);
    setNavigationForwardHistory([]);
    setSymbolRestore(undefined);
    currentSymbol.current = undefined;
  };
  const requestCloseTab = (
    key: string,
    anchor: HTMLElement,
  ): void => {
    navigationHaptic();
    setTabCloseRequest({ kind: "one", key, anchor });
  };
  const confirmTabCloseRequest = (): void => {
    const request = tabCloseRequest;
    if (!request) return;
    setTabCloseRequest(undefined);
    importantHaptic();
    if (request.kind === "one") closeTab(request.key);
    else if (request.kind === "others") closeOtherTabs(request.key);
    else closeAllTabs();
  };
  const requestedTab = tabCloseRequest?.kind === "one"
    ? tabs.find((tab) => reviewTabKey(tab) === tabCloseRequest.key)
    : undefined;
  const tabCloseCount = tabCloseRequest?.kind === "others"
    ? modeTabs.filter((tab) =>
      reviewTabKey(tab) !== tabCloseRequest.key && !tab.pinned
    ).length
    : tabCloseRequest?.kind === "all"
    ? modeTabs.length
    : 1;
  const tabCloseTitle = tabCloseRequest?.kind === "one"
    ? `Close ${requestedTab?.path.split("/").at(-1) ?? "this tab"}?`
    : tabCloseRequest?.kind === "others"
    ? `Close ${tabCloseCount} other ${tabCloseCount === 1 ? "tab" : "tabs"}?`
    : tabCloseCount === 1
    ? "Close the only open tab?"
    : `Close all ${tabCloseCount} tabs?`;
  const tabCloseDetail = tabCloseRequest?.kind === "others"
    ? "Other unpinned tabs will close. Pinned tabs stay open."
    : tabCloseRequest?.kind === "all"
    ? "Every open file tab will close."
    : "This tab will close.";
  const tabCloseAction = tabCloseRequest?.kind === "others"
    ? "Close others"
    : tabCloseRequest?.kind === "all"
    ? "Close all"
    : "Close tab";
  const reviewIndex = target.kind === "diff"
    ? target.queue.findIndex((entry) =>
      entry.change.path === target.path && entry.scope === target.scope
    )
    : -1;
  useEffect(() => {
    if (target.kind !== "diff") return;
    const stillChanged = gitQueue.some((entry) =>
      entry.change.path === target.path && entry.scope === target.scope
    );
    if (!stillChanged) {
      setDiffTarget(undefined);
      setCurrentRevision(undefined);
    }
  }, [gitQueue, target]);
  const moveReview = (offset: number): void => {
    if (target.kind !== "diff") return;
    const entry = target.queue[reviewIndex + offset];
    if (entry) openDiff(entry, target.queue);
  };
  const targetReviewKey = target.kind === "diff"
    ? reviewEntryKey(target.path, target.scope)
    : undefined;
  const targetIsReviewed = targetReviewKey
    ? revisionMatches(reviewProgress, targetReviewKey, currentRevision)
    : false;
  useEffect(() => {
    if (
      !workspace?.sessionId ||
      !targetReviewKey ||
      !currentRevision ||
      reviewProgress[targetReviewKey] === undefined ||
      targetIsReviewed
    ) {
      return;
    }
    const next = { ...reviewProgress };
    delete next[targetReviewKey];
    setReviewProgress(next);
    mutateMobileReview(workspace.sessionId, "markReviewed", {
      key: targetReviewKey,
      revision: null,
    });
  }, [
    currentRevision,
    reviewProgress,
    targetIsReviewed,
    targetReviewKey,
    workspace?.sessionId,
  ]);
  const toggleReviewed = (): void => {
    if (!targetReviewKey || !currentRevision || !workspace?.sessionId) return;
    const next = { ...reviewProgress };
    if (targetIsReviewed) delete next[targetReviewKey];
    else next[targetReviewKey] = currentRevision;
    setReviewProgress(next);
    mutateMobileReview(workspace.sessionId, "markReviewed", {
      key: targetReviewKey,
      revision: targetIsReviewed ? null : currentRevision,
    });
  };
  const currentSession = sessions.find((session) =>
    session.id === workspace?.sessionId
  );
  const currentProject = repositoryContext?.project ??
    (currentSession ? sessionProject(currentSession) : undefined);
  // Match the primary mobile Sessions rail: server order is newest-first, while
  // the physical list reads oldest-to-newest from top to bottom and opens at
  // its live edge. Users move upward to older sessions.
  const displayedSessions = [...sessions].reverse();
  const projectSessions = displayedSessions.filter((session) =>
    sessionProject(session) === currentProject
  );
  const otherSessions = displayedSessions.filter((session) =>
    sessionProject(session) !== currentProject
  );
  useEffect(() => {
    if (!sessionSwitcherOpen) return undefined;
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => {
        const list = sessionListRef.current;
        if (list) list.scrollTop = list.scrollHeight - list.clientHeight;
      });
    });
    return () => cancelAnimationFrame(frame);
  }, [sessionSwitcherOpen]);
  const switchSession = (session: SessionMeta): void => {
    navigationHaptic();
    setActiveSessionId(session.id);
    openSession(session.id);
    setSessionSwitcherOpen(false);
  };

  return (
    <ReviewDrawerShell
      onOpenChange={handleDrawerOpenChange}
      closeRequest={closeRequest}
      toggleRequest={toggleDrawerRequest}
      drawer={mode === "files"
        ? (
          <ReviewFileTree
            sessionId={workspace?.sessionId}
            cwd={workspace?.cwd}
            onOpenFile={openSource}
            currentPath={sourceTarget?.path}
            onClose={() => setCloseRequest((value) => value + 1)}
            refreshToken={dataRevision}
          />
        )
        : (
          <ReviewRepository
            key={`${workspace?.sessionId ?? "none"}:${dataRevision}`}
            sessionId={workspace?.sessionId}
            machineLabel={currentSession?.machine_id ?? "hawk"}
            onOpenDiff={openDiff}
            reviewed={new Set(Object.keys(reviewProgress))}
            onRevision={adoptManifestRevision}
            onClose={() => setCloseRequest((value) => value + 1)}
            refreshToken={dataRevision}
          />
        )}
    >
      <Stack
        sx={{
          height: "100%",
          minWidth: 0,
          bgcolor: "background.default",
          color: "text.primary",
          pt: "env(safe-area-inset-top, 0px)",
        }}
      >
        <Stack
          direction="row"
          alignItems="center"
          sx={{
            minHeight: 52,
            px: 2,
            borderBottom: 1,
            borderColor: "divider",
          }}
        >
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography
              variant="body2"
              fontFamily="var(--cowboy-font-mono)"
              noWrap
            >
              {target.kind === "changes"
                ? mode === "files" ? "Worktree" : "Changes"
                : target.path}
            </Typography>
            <Box
              component="button"
              type="button"
              aria-label={`Switch session. Current session ${
                currentSession?.title ?? "none"
              }`}
              onClick={() => {
                navigationHaptic();
                setSessionSwitcherOpen(true);
              }}
              sx={{
                display: "flex",
                alignItems: "center",
                gap: 0.625,
                minWidth: 0,
                maxWidth: "100%",
                p: 0,
                border: 0,
                bgcolor: "transparent",
                color: "text.secondary",
                font: "inherit",
                textAlign: "left",
                cursor: "pointer",
                WebkitTapHighlightColor: "transparent",
                "&:active": { color: "primary.main" },
              }}
            >
              <Box
                aria-hidden
                sx={{
                  width: 6,
                  height: 6,
                  flex: "0 0 auto",
                  borderRadius: "50%",
                  bgcolor: currentSession
                    ? sessionStatusColor(currentSession.status)
                    : "text.disabled",
                }}
              />
              <Typography variant="caption" noWrap>
                {target.kind === "diff"
                  ? target.scope === "staged"
                    ? "Staged · "
                    : target.scope === "unstaged"
                    ? "Unstaged · "
                    : "Conflict · "
                  : ""}
                {currentSession?.title ?? currentProject ??
                  (workspace ? "Loading session…" : "Choose session")}
                {currentProject && currentSession?.title !== currentProject
                  ? ` · ${currentProject}`
                  : ""}
                {repositoryContext?.branch
                  ? ` · ${repositoryContext.branch}`
                  : ""}
              </Typography>
              <KeyboardArrowDown
                aria-hidden
                sx={{ fontSize: "0.9rem", flex: "0 0 auto" }}
              />
            </Box>
          </Box>
          {mode === "files" &&
            (navigationHistory.length > 0 ||
              navigationForwardHistory.length > 0) &&
            (
              <Stack
                direction="row"
                role="group"
                aria-label="Code navigation history"
              >
                <IconButton
                  aria-label="Back to previous code location"
                  disabled={navigationHistory.length === 0}
                  onClick={navigateBack}
                >
                  <ArrowBack />
                </IconButton>
                <IconButton
                  aria-label="Forward to next code location"
                  disabled={navigationForwardHistory.length === 0}
                  onClick={navigateForward}
                >
                  <ArrowForward />
                </IconButton>
              </Stack>
            )}
          {target.kind === "diff" && (
            <IconButton
              aria-label={targetIsReviewed
                ? "Mark unreviewed"
                : "Mark reviewed"}
              color={targetIsReviewed ? "success" : "default"}
              disabled={!currentRevision}
              onClick={toggleReviewed}
            >
              {targetIsReviewed ? <CheckCircle /> : <CheckCircleOutline />}
            </IconButton>
          )}
        </Stack>
        {!workspace
          ? (
            <Stack
              component="main"
              alignItems="center"
              justifyContent="center"
              spacing={1.5}
              sx={{ flex: 1, px: 4, textAlign: "center" }}
            >
              <FolderOpenOutlined color="disabled" />
              <Typography color="text.secondary">
                Select an Agent session to review its worktree
              </Typography>
            </Stack>
          )
          : target.kind === "changes"
          ? (
            <Stack
              component="main"
              alignItems="center"
              justifyContent="center"
              spacing={1.5}
              sx={{ flex: 1, px: 4, textAlign: "center" }}
            >
              {mode === "git"
                ? <DifferenceOutlined color="disabled" />
                : <FolderOpenOutlined color="disabled" />}
              <Typography color="text.secondary">
                {mode === "git"
                  ? "Select a changed file from Git review"
                  : "Select a file from the Worktree"}
              </Typography>
            </Stack>
          )
          : (
            <DocumentView
              sessionId={workspace.sessionId}
              target={target}
              onRevision={setCurrentRevision}
              markdownPreview={target.kind === "source" &&
                isMarkdownReviewPath(target.path) && markdownPreview}
              languageData={languageData?.path === target.path
                ? languageData
                : undefined}
              restoreSymbol={symbolRestore}
              closeSymbolRequest={closeSymbolRequest}
              onRestoreSymbolConsumed={(id) => {
                setSymbolRestore((request) =>
                  request?.id === id ? undefined : request
                );
              }}
              onSymbolOpenChange={(point) => {
                currentSymbol.current = point;
                const frame = currentNavigationFrame.current;
                if (frame && frame.path === target.path) {
                  const { symbol: _symbol, ...frameWithoutSymbol } = frame;
                  currentNavigationFrame.current = {
                    ...frameWithoutSymbol,
                    ...(point ? { symbol: point } : {}),
                  };
                }
              }}
              onVisibleSourceLine={(line) => {
                currentVisibleLine.current = line;
                const frame = currentNavigationFrame.current;
                if (frame && frame.path === target.path) {
                  currentNavigationFrame.current = {
                    ...frame,
                    revealLine: line,
                  };
                }
              }}
              onBufferUnavailable={requestBufferRecovery}
              onNavigate={(location, origin) => {
                if (target.kind !== "source") return;
                const previous: CodeNavigationEntry = {
                  kind: "source",
                  path: target.path,
                  revealLine: currentVisibleLine.current ?? origin.row + 1,
                  symbol: origin,
                };
                setNavigationHistory((history) =>
                  [...history, previous].slice(-32)
                );
                setNavigationForwardHistory([]);
                setSymbolRestore(undefined);
                setCloseSymbolRequest((value) => value + 1);
                currentSymbol.current = undefined;
                currentNavigationFrame.current = {
                  kind: "source",
                  path: location.path,
                  revealLine: location.start.row + 1,
                  navigationRange: {
                    start: location.start,
                    end: location.end,
                  },
                };
                openSource(location.path, location.start.row + 1, true, {
                  start: location.start,
                  end: location.end,
                });
              }}
            />
          )}
        {workspace && target.kind === "source" && (
          <ReviewOutline
            open={outlineOpen}
            onClose={() => setOutlineOpen(false)}
            sessionId={workspace.sessionId}
            path={target.path}
            currentLine={syncedReview.positions?.[target.path]?.line}
            onSelect={(line) => {
              setMarkdownPreview(false);
              setSourceTarget({
                kind: "source",
                path: target.path,
                revealLine: line,
                revealRequestId: ++revealRangeId.current,
              });
            }}
          />
        )}
        <Box
          sx={{
            flexShrink: 0,
            borderTop: 1,
            borderColor: "divider",
            bgcolor: (theme) =>
              alpha(
                theme.palette.background.default,
                theme.palette.mode === "dark" ? 0.72 : 0.76,
              ),
            backdropFilter: "blur(30px) saturate(200%)",
            WebkitBackdropFilter: "blur(30px) saturate(200%)",
            boxShadow: (theme) => theme.palette.mode === "dark"
              ? "0 -1px 24px rgba(0,0,0,0.5)"
              : "0 -1px 24px rgba(0,0,0,0.07)",
          }}
        >
          <ReviewTabStrip
            tabs={modeTabs}
            activeKey={activeTabKey}
            onActivate={activateOrCollapseTab}
            onClose={requestCloseTab}
            onCloseOthers={(key, anchor) => {
              navigationHaptic();
              setTabCloseRequest({ kind: "others", key, anchor });
            }}
            onCloseAll={(anchor) => {
              navigationHaptic();
              setTabCloseRequest({ kind: "all", anchor });
            }}
            onTogglePin={(key) => {
              const source = tabs.find((tab) =>
                tab.kind === "source" && reviewTabKey(tab) === key
              );
              if (source?.kind === "source" && workspace?.sessionId) {
                mutateMobileReview(workspace.sessionId, "setPinned", {
                  path: source.path,
                  pinned: !source.pinned,
                });
              }
              setTabs((current) => {
                if (current.some((tab) => reviewTabKey(tab) === key)) {
                  return toggleReviewTabPin(current, key);
                }
                const gitTab = gitTabs.find((tab) => reviewTabKey(tab) === key);
                return gitTab
                  ? [...current, { ...gitTab, pinned: true }]
                  : current;
              });
            }}
            allowCloseActions={mode === "files"}
            allowReorder={mode === "files"}
            onReorder={(movingKey, targetKey) => {
              const next = reorderReviewTabs(tabs, movingKey, targetKey);
              setTabs(next);
              if (workspace?.sessionId) {
                mutateMobileReview(workspace.sessionId, "reorder", {
                  paths: next.flatMap((tab) =>
                    tab.kind === "source" ? [tab.path] : []
                  ),
                });
              }
            }}
          />
          <Box
            component="nav"
            aria-label="Code Review controls"
            sx={{
              pb: "max(calc(env(safe-area-inset-bottom) - 18px), 12px)",
              pl: "env(safe-area-inset-left, 0px)",
              pr: "max(env(safe-area-inset-right, 0px), 10px)",
            }}
          >
            <Toolbar
              variant="dense"
              sx={{
                minHeight: 44,
                "@media (min-width: 600px)": { minHeight: 44 },
              }}
            >
              <ReviewSettings language={language} />
              <Button
                variant="text"
                color="inherit"
                size="small"
                aria-label={`Switch to ${
                  mode === "git" ? "Files" : "Git review"
                }`}
                startIcon={mode === "git"
                  ? <DifferenceOutlined fontSize="small" />
                  : <FolderOutlined fontSize="small" />}
                onClick={() => {
                  navigationHaptic();
                  const nextMode = mode === "git" ? "files" : "git";
                  setMode(nextMode);
                  if (workspace?.sessionId) {
                    mutateMobileReview(workspace.sessionId, "setMode", {
                      mode: nextMode,
                    });
                  }
                }}
                sx={{
                  ml: 0.25,
                  minWidth: 0,
                  height: 40,
                  px: 1,
                  borderRadius: 2,
                  color: "text.secondary",
                  textTransform: "none",
                  fontSize: "0.75rem",
                  fontWeight: 600,
                  "& .MuiButton-startIcon": {
                    mr: 0.625,
                  },
                }}
              >
                {mode === "git" ? "Changes" : "Files"}
              </Button>
              {target.kind !== "changes" && !(
                target.kind === "source" &&
                isMarkdownReviewPath(target.path) &&
                markdownPreview
              ) && (
                <IconButton
                  aria-label={settings.softWrap
                    ? "Disable line wrapping"
                    : "Enable line wrapping"}
                  color={settings.softWrap ? "primary" : "default"}
                  onClick={() =>
                    updateReviewSettings({ softWrap: !settings.softWrap })}
                >
                  <WrapText />
                </IconButton>
              )}
              {target.kind === "source" && isMarkdownReviewPath(target.path) &&
                (
                  <IconButton
                    aria-label={markdownPreview
                      ? "Show Markdown source"
                      : "Preview Markdown"}
                    aria-pressed={markdownPreview}
                    color={markdownPreview ? "primary" : "default"}
                    onClick={() => setMarkdownPreview((value) => !value)}
                  >
                    <VisibilityOutlined />
                  </IconButton>
                )}
              {target.kind === "diff" && (
                <>
                  <Box sx={{ flex: 1 }} />
                  <IconButton
                    aria-label="Previous change"
                    disabled={reviewIndex <= 0}
                    onClick={() => moveReview(-1)}
                  >
                    <ChevronLeft />
                  </IconButton>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ minWidth: 52, textAlign: "center" }}
                  >
                    {reviewIndex + 1} / {target.queue.length}
                  </Typography>
                  <IconButton
                    aria-label="Next change"
                    disabled={reviewIndex >= target.queue.length - 1}
                    onClick={() => moveReview(1)}
                  >
                    <ChevronRight />
                  </IconButton>
                </>
              )}
              <Box sx={{ flex: 1 }} />
              {target.kind === "source" && (
                <IconButton
                  aria-label="Open file outline"
                  aria-pressed={outlineOpen}
                  color={outlineOpen ? "primary" : "default"}
                  onClick={() => {
                    navigationHaptic();
                    setOutlineOpen(true);
                  }}
                >
                  <FormatListBulleted />
                </IconButton>
              )}
              <IconButton
                aria-label={drawerOpen
                  ? `Close ${
                    mode === "git" ? "Git changes" : "file tree"
                  } sidebar`
                  : `Open ${
                    mode === "git" ? "Git changes" : "file tree"
                  } sidebar`}
                aria-pressed={drawerOpen}
                sx={{
                  color: "text.primary",
                  bgcolor: drawerOpen ? "action.selected" : "transparent",
                }}
                onClick={() => setToggleDrawerRequest((value) => value + 1)}
              >
                {mode === "git"
                  ? (
                    <Badge
                      variant="dot"
                      color="primary"
                      invisible={changeCount === 0}
                      slotProps={{
                        badge: {
                          "aria-label": `${changeCount} changed files`,
                        },
                      }}
                      sx={{
                        "& .MuiBadge-badge": {
                          top: 3,
                          right: 2,
                          minWidth: 7,
                          width: 7,
                          height: 7,
                        },
                      }}
                    >
                      <DifferenceOutlined />
                    </Badge>
                  )
                  : <AccountTreeOutlined />}
              </IconButton>
            </Toolbar>
          </Box>
        </Box>
        <Popover
          open={tabCloseRequest !== undefined}
          anchorEl={tabCloseRequest?.anchor}
          onClose={() => setTabCloseRequest(undefined)}
          anchorOrigin={{ vertical: "top", horizontal: "right" }}
          transformOrigin={{ vertical: "bottom", horizontal: "right" }}
          slotProps={{
            paper: {
              sx: {
                width: "min(300px, calc(100vw - 32px))",
                borderRadius: 2.5,
              },
            },
          }}
        >
          <Box data-review-tab-close-confirm sx={{ p: 1.5 }}>
            <Typography variant="body2" fontWeight={700}>
              {tabCloseTitle}
            </Typography>
            <Typography
              variant="body2"
              color="text.secondary"
              sx={{ mt: 0.5 }}
            >
              {tabCloseDetail} Files and their contents are not deleted.
            </Typography>
            <Stack
              direction="row"
              spacing={1}
              justifyContent="flex-end"
              sx={{ mt: 1.5 }}
            >
              <Button
                size="small"
                color="inherit"
                onClick={() => setTabCloseRequest(undefined)}
                sx={{ textTransform: "none" }}
              >
                Cancel
              </Button>
              <Button
                size="small"
                color="error"
                variant="contained"
                startIcon={<Close />}
                onClick={confirmTabCloseRequest}
                sx={{ textTransform: "none" }}
              >
                {tabCloseAction}
              </Button>
            </Stack>
          </Box>
        </Popover>
        <Sheet
          open={sessionSwitcherOpen}
          onClose={() => {
            setSessionSwitcherOpen(false);
          }}
          title="Sessions"
          forceSheet
          cover
        >
          <Stack
            ref={sessionListRef}
            spacing={1.5}
            sx={{
              height: "calc(100dvh - 148px)",
              minHeight: 0,
              overflowY: "auto",
              overscrollBehavior: "contain",
              pb: "calc(88px + env(safe-area-inset-bottom, 0px))",
            }}
          >
            {[
              { label: "Other sessions", sessions: otherSessions },
              { label: "Current project", sessions: projectSessions },
            ].map((section) =>
              section.sessions.length > 0 && (
                <Stack key={section.label} spacing={0.5}>
                  <Typography
                    variant="overline"
                    color="text.secondary"
                    sx={{ px: 1.25, fontWeight: 700, letterSpacing: "0.09em" }}
                  >
                    {section.label}
                  </Typography>
                  <Stack divider={<Divider flexItem />}>
                    {section.sessions.map((session) => {
                      const selected = session.id === workspace?.sessionId;
                      const project = sessionProject(session);
                      return (
                        <ListItemButton
                          key={session.id}
                          selected={selected}
                          onClick={() => switchSession(session)}
                          sx={{
                            minHeight: 68,
                            px: 1.25,
                            py: 0.75,
                            borderRadius: 1.5,
                            gap: 1.25,
                          }}
                        >
                          <Box
                            aria-label={`${session.status} session`}
                            sx={{
                              width: 8,
                              height: 8,
                              flex: "0 0 auto",
                              borderRadius: "50%",
                              bgcolor: sessionStatusColor(session.status),
                            }}
                          />
                          <Box sx={{ flex: 1, minWidth: 0 }}>
                            <Stack
                              direction="row"
                              spacing={0.75}
                              alignItems="baseline"
                            >
                              <Typography
                                variant="body2"
                                fontWeight={selected ? 700 : 600}
                                noWrap
                              >
                                {session.title}
                              </Typography>
                              {selected && (
                                <Typography
                                  variant="caption"
                                  color="primary.main"
                                >
                                  Current
                                </Typography>
                              )}
                            </Stack>
                            <Stack spacing={0.125} sx={{ mt: 0.125 }}>
                              <Typography
                                variant="caption"
                                color="text.secondary"
                                noWrap
                              >
                                {session.machine_id ?? "local"} ·{" "}
                                {session.provider} · {project}
                              </Typography>
                              <Typography
                                variant="caption"
                                color="text.disabled"
                                noWrap
                              >
                                {session.cwd}
                              </Typography>
                            </Stack>
                          </Box>
                          <ChevronRight color="disabled" fontSize="small" />
                        </ListItemButton>
                      );
                    })}
                  </Stack>
                </Stack>
              )
            )}
            {displayedSessions.length === 0 && (
              <Typography
                color="text.secondary"
                variant="body2"
                sx={{ py: 3, textAlign: "center" }}
              >
                No matching sessions
              </Typography>
            )}
          </Stack>
        </Sheet>
      </Stack>
    </ReviewDrawerShell>
  );
}
