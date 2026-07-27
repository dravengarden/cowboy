import {
  ArrowBack,
  CheckCircle,
  CheckCircleOutline,
  ChevronLeft,
  ChevronRight,
  FolderOpenOutlined,
  KeyboardArrowDown,
  KeyboardArrowUp,
  ViewSidebarOutlined,
  WrapText,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  IconButton,
  Stack,
  Toolbar,
  Typography,
} from "@mui/material";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { useActiveWorkspaceBinding } from "../../controlPlane";
import {
  CodeApiError,
  fetchCodeDiffPage,
  fetchCodeFile,
  fetchCodeFilePage,
  fetchCodeManifest,
} from "./codeApi";
import { invalidateDiffCache, loadCodeDiff } from "./diffCache";
import { diffHunkLines, reviewEntryKey } from "./diffNavigationModel";
import {
  loadReviewProgress,
  type ReviewProgress,
  revisionMatches,
  saveReviewProgress,
} from "./reviewProgress";
import { ReviewChanges } from "./ReviewChanges";
import { ReviewDrawerShell } from "./ReviewDrawerShell";
import { ReviewFileTree } from "./ReviewFileTree";
import { ReviewSettings } from "./ReviewSettings";
import {
  updateReviewSettings,
  useReviewSettings,
} from "./reviewSettings";
import type { CodeDiffScope } from "./codeApi";
import type { GitReviewEntry } from "./gitReviewModel";

const CodeViewer = lazy(() => import("./CodeViewer"));

type ReviewTarget =
  | { kind: "changes" }
  | {
    kind: "diff";
    path: string;
    scope: CodeDiffScope;
    queue: GitReviewEntry[];
  }
  | { kind: "source"; path: string };

function DocumentView({
  sessionId,
  target,
  onRevision,
}: {
  sessionId: string;
  target: Exclude<ReviewTarget, { kind: "changes" }>;
  onRevision: (revision: string | undefined) => void;
}): React.JSX.Element {
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
  const [error, setError] = useState(false);
  const [hunkIndex, setHunkIndex] = useState(0);
  const hunks = target.kind === "diff" ? diffHunkLines(text) : [];

  useEffect(() => {
    const controller = new AbortController();
    const diffTarget = target.kind === "diff" ? target : undefined;
    setLoading(true);
    setError(false);
    setText("");
    setCounts(undefined);
    setLimited(false);
    setNextCursor(undefined);
    setRevision(undefined);
    setLoadMoreError(false);
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
      setText(result.text);
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
      if (!(reason instanceof DOMException && reason.name === "AbortError")) {
        setError(true);
      }
    }).finally(() => {
      if (!controller.signal.aborted) setLoading(false);
    });
    return () => {
      controller.abort();
      pageController.current?.abort();
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
    return (
      <Alert severity="error" sx={{ m: 2 }}>
        {target.kind === "diff"
          ? "This diff could not be loaded"
          : "This file could not be loaded"}
      </Alert>
    );
  }
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
              startIcon={loadingMore ? <CircularProgress size={12} /> : undefined}
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
      <Box sx={{ flex: 1, minHeight: 0 }}>
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
            revealLine={target.kind === "diff" ? hunks[hunkIndex] : undefined}
          />
        </Suspense>
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
  const settings = useReviewSettings();
  const [target, setTarget] = useState<ReviewTarget>({ kind: "changes" });
  const [closeRequest, setCloseRequest] = useState(0);
  const [toggleDrawerRequest, setToggleDrawerRequest] = useState(0);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [reviewProgress, setReviewProgress] = useState<ReviewProgress>({});
  const [currentRevision, setCurrentRevision] = useState<string>();
  const [dataRevision, setDataRevision] = useState(0);
  const manifestRevision = useRef<string | undefined>(undefined);
  const adoptManifestRevision = useCallback((revision: string): void => {
    manifestRevision.current = revision;
  }, []);
  const handleDrawerOpenChange = useCallback((open: boolean): void => {
    setDrawerOpen(open);
    onDrawerOpenChange(open);
  }, [onDrawerOpenChange]);

  useEffect(() => {
    manifestRevision.current = undefined;
    setDataRevision(0);
  }, [workspace?.sessionId]);

  useEffect(() => {
    if (!active || !workspace?.sessionId) return undefined;
    let controller: AbortController | undefined;
    const refreshManifest = (): void => {
      controller?.abort();
      controller = new AbortController();
      void fetchCodeManifest(workspace.sessionId, controller.signal)
        .then((manifest) => {
          const previous = manifestRevision.current;
          manifestRevision.current = manifest.revision;
          if (previous && previous !== manifest.revision) {
            invalidateDiffCache(workspace.sessionId);
            setDataRevision((value) => value + 1);
          }
        })
        // The ordinary tree/changes error surfaces remain authoritative.
        .catch(() => undefined);
    };
    const timer = globalThis.setInterval(refreshManifest, 5_000);
    return () => {
      globalThis.clearInterval(timer);
      controller?.abort();
    };
  }, [active, workspace?.sessionId]);

  useEffect(() => {
    setTarget({ kind: "changes" });
    setCurrentRevision(undefined);
    setReviewProgress(
      workspace?.sessionId ? loadReviewProgress(workspace.sessionId) : {},
    );
  }, [workspace?.sessionId]);

  const openSource = (path: string): void => {
    setCurrentRevision(undefined);
    setTarget({ kind: "source", path });
    setCloseRequest((value) => value + 1);
  };
  const openDiff = (
    entry: GitReviewEntry,
    queue: GitReviewEntry[],
  ): void => {
    setCurrentRevision(undefined);
    setTarget({
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      queue,
    });
  };
  const reviewIndex = target.kind === "diff"
    ? target.queue.findIndex((entry) =>
      entry.change.path === target.path && entry.scope === target.scope
    )
    : -1;
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
    saveReviewProgress(workspace.sessionId, next);
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
    saveReviewProgress(workspace.sessionId, next);
  };

  return (
    <ReviewDrawerShell
      onOpenChange={handleDrawerOpenChange}
      closeRequest={closeRequest}
      toggleRequest={toggleDrawerRequest}
      drawer={
        <ReviewFileTree
          sessionId={workspace?.sessionId}
          cwd={workspace?.cwd}
          onOpenFile={openSource}
          currentPath={target.kind === "changes" ? undefined : target.path}
          onClose={() => setCloseRequest((value) => value + 1)}
          refreshToken={dataRevision}
        />
      }
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
        {target.kind !== "changes" && (
          <Stack
            direction="row"
            alignItems="center"
            sx={{
              minHeight: 44,
              px: 0.5,
              borderBottom: 1,
              borderColor: "divider",
            }}
          >
            <IconButton
              aria-label="Back to changes"
              onClick={() => setTarget({ kind: "changes" })}
            >
              <ArrowBack />
            </IconButton>
            <Box sx={{ flex: 1, minWidth: 0 }}>
              <Typography
                variant="body2"
                fontFamily="var(--cowboy-font-mono)"
                noWrap
              >
                {target.path}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {target.kind === "diff"
                  ? target.scope === "staged"
                    ? "Staged changes"
                    : target.scope === "unstaged"
                    ? "Unstaged changes"
                    : "Conflict review"
                  : "Read-only file"}
              </Typography>
            </Box>
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
        )}
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
            <ReviewChanges
              key={`${workspace.sessionId}:${dataRevision}`}
              sessionId={workspace.sessionId}
              onOpenDiff={openDiff}
              reviewed={new Set(Object.keys(reviewProgress))}
              onRevision={adoptManifestRevision}
              onRefresh={() => {
                setReviewProgress({});
                saveReviewProgress(workspace.sessionId, {});
              }}
            />
          )
          : (
            <DocumentView
              sessionId={workspace.sessionId}
              target={target}
              onRevision={setCurrentRevision}
            />
          )}
        <Box
          component="nav"
          aria-label="Code Review controls"
          sx={{
            pb: "max(calc(env(safe-area-inset-bottom) - 18px), 12px)",
            pl: "env(safe-area-inset-left, 0px)",
            pr: "env(safe-area-inset-right, 0px)",
            borderTop: 1,
            borderColor: "divider",
            bgcolor: "background.default",
          }}
        >
          <Toolbar
            variant="dense"
            sx={{
              minHeight: 44,
              "@media (min-width: 600px)": { minHeight: 44 },
            }}
            >
            <ReviewSettings />
            {target.kind !== "changes" && (
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
            <IconButton
              aria-label={drawerOpen
                ? "Close worktree sidebar"
                : "Open worktree sidebar"}
              color={drawerOpen ? "primary" : "default"}
              onClick={() => setToggleDrawerRequest((value) => value + 1)}
            >
              <ViewSidebarOutlined />
            </IconButton>
          </Toolbar>
        </Box>
      </Stack>
    </ReviewDrawerShell>
  );
}
