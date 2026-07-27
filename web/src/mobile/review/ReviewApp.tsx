import {
  ArrowBack,
  CheckCircle,
  CheckCircleOutline,
  ChevronLeft,
  ChevronRight,
  DifferenceOutlined,
  FolderOpenOutlined,
  KeyboardArrowDown,
  KeyboardArrowUp,
  VisibilityOutlined,
  ViewSidebarOutlined,
  WrapText,
} from "@mui/icons-material";
import {
  Alert,
  Badge,
  Box,
  Button,
  Chip,
  CircularProgress,
  IconButton,
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
import { useActiveWorkspaceBinding } from "../../controlPlane";
import { Markdown } from "../../Markdown";
import {
  CodeApiError,
  closeCodeBuffer,
  fetchCodeDiffPage,
  fetchCodeFile,
  fetchCodeFilePage,
  fetchCodeChanges,
  fetchCodeLanguage,
  fetchCodeManifest,
  openCodeBuffer,
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
import { isMarkdownReviewPath } from "./reviewMarkdown";
import {
  updateReviewSettings,
  useReviewSettings,
} from "./reviewSettings";
import type { CodeDiffScope } from "./codeApi";
import type { CodeLanguage } from "./codeApi";
import type { GitReviewEntry } from "./gitReviewModel";
import { groupGitChanges, reviewQueue } from "./gitReviewModel";
import { ReviewTabStrip } from "./ReviewTabStrip";
import {
  closeOtherReviewTabs,
  closeReviewTab,
  loadReviewTabs,
  openReviewTab,
  reviewTabKey,
  saveReviewTabs,
  toggleReviewTabPin,
  type ReviewTab,
} from "./reviewTabs";

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

type ReviewMode = "code" | "git";

function DocumentView({
  sessionId,
  target,
  onRevision,
  markdownPreview,
  languageData,
}: {
  sessionId: string;
  target: Exclude<ReviewTarget, { kind: "changes" }>;
  onRevision: (revision: string | undefined) => void;
  markdownPreview: boolean;
  languageData?: CodeLanguage | undefined;
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
                "& hr": { border: 0, borderTop: 1, borderColor: "divider", my: 2 },
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
                revealLine={target.kind === "diff" ? hunks[hunkIndex] : undefined}
                languageData={target.kind === "source" ? languageData : undefined}
                diagnostics={settings.diagnostics}
                inlayHints={settings.inlayHints}
                semanticHighlighting={settings.semanticHighlighting}
              />
            </Suspense>
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
  const settings = useReviewSettings();
  const [mode, setMode] = useState<ReviewMode>("git");
  const [markdownPreview, setMarkdownPreview] = useState(true);
  const [sourceTarget, setSourceTarget] = useState<
    Extract<ReviewTarget, { kind: "source" }> | undefined
  >();
  const [diffTarget, setDiffTarget] = useState<
    Extract<ReviewTarget, { kind: "diff" }> | undefined
  >();
  const target: ReviewTarget = mode === "code"
    ? sourceTarget ?? { kind: "changes" }
    : diffTarget ?? { kind: "changes" };
  const leasedPath = target.kind === "changes" ? undefined : target.path;
  const [closeRequest, setCloseRequest] = useState(0);
  const [toggleDrawerRequest, setToggleDrawerRequest] = useState(0);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [reviewProgress, setReviewProgress] = useState<ReviewProgress>({});
  const [currentRevision, setCurrentRevision] = useState<string>();
  const [dataRevision, setDataRevision] = useState(0);
  const [changeCount, setChangeCount] = useState(0);
  const [language, setLanguage] = useState<
    import("./codeApi").CodeLanguageCapabilities
  >();
  const [languageData, setLanguageData] = useState<CodeLanguage>();
  const [tabs, setTabs] = useState<ReviewTab[]>([]);
  const [tabsReadySession, setTabsReadySession] = useState<string>();
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
      !active ||
      !workspace?.sessionId ||
      !leasedPath
    ) {
      setLanguageData(undefined);
      return undefined;
    }
    const sessionId = workspace.sessionId;
    const path = leasedPath;
    const leaseId = crypto.randomUUID();
    let released = false;
    let opened = false;
    const release = (): void => {
      if (!opened) return;
      opened = false;
      void closeCodeBuffer(sessionId, path, leaseId).catch(() => undefined);
    };
    void openCodeBuffer(sessionId, path, leaseId)
      .then(() => {
        opened = true;
        return fetchCodeLanguage(sessionId, path);
      })
      .then((value) => {
        if (!released) setLanguageData(value);
        if (released) release();
      })
      // File rendering remains useful when language intelligence is unavailable.
      .catch(() => undefined);
    return () => {
      released = true;
      setLanguageData(undefined);
      release();
    };
  }, [active, leasedPath, workspace?.sessionId]);

  useEffect(() => {
    setTabsReadySession(undefined);
    manifestRevision.current = undefined;
    setDataRevision(0);
    setChangeCount(0);
    setLanguage(undefined);
  }, [workspace?.sessionId]);

  useEffect(() => {
    if (!active || !workspace?.sessionId) return undefined;
    let controller: AbortController | undefined;
    const refreshManifest = (): void => {
      controller?.abort();
      controller = new AbortController();
      void fetchCodeManifest(workspace.sessionId, controller.signal)
        .then((manifest) => {
          setChangeCount(manifest.changeCount);
          setLanguage(manifest.language);
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
    refreshManifest();
    const timer = globalThis.setInterval(refreshManifest, 5_000);
    return () => {
      globalThis.clearInterval(timer);
      controller?.abort();
    };
  }, [active, workspace?.sessionId]);

  useEffect(() => {
    setSourceTarget(undefined);
    setDiffTarget(undefined);
    setCurrentRevision(undefined);
    setReviewProgress(
      workspace?.sessionId ? loadReviewProgress(workspace.sessionId) : {},
    );
    setTabs(workspace?.sessionId ? loadReviewTabs(workspace.sessionId) : []);
    setTabsReadySession(workspace?.sessionId);
  }, [workspace?.sessionId]);

  const openSource = (path: string): void => {
    setCurrentRevision(undefined);
    setMode("code");
    setMarkdownPreview(isMarkdownReviewPath(path));
    setSourceTarget({ kind: "source", path });
    setTabs((current) =>
      openReviewTab(current, { kind: "source", path, pinned: false })
    );
    setCloseRequest((value) => value + 1);
  };
  const openDiff = (
    entry: GitReviewEntry,
    queue: GitReviewEntry[],
  ): void => {
    setCurrentRevision(undefined);
    setMode("git");
    setDiffTarget({
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      queue,
    });
    setTabs((current) =>
      openReviewTab(current, {
        kind: "diff",
        path: entry.change.path,
        scope: entry.scope,
        pinned: false,
      })
    );
  };
  useEffect(() => {
    if (workspace?.sessionId === tabsReadySession && tabsReadySession) {
      saveReviewTabs(tabsReadySession, tabs);
    }
  }, [tabs, tabsReadySession, workspace?.sessionId]);
  const activateTab = useCallback(async (tab: ReviewTab): Promise<void> => {
    if (tab.kind === "source") {
      openSource(tab.path);
      return;
    }
    if (!workspace?.sessionId) return;
    try {
      const changes = await fetchCodeChanges(workspace.sessionId);
      const queue = reviewQueue(groupGitChanges(changes.changes));
      const entry = queue.find((candidate) =>
        candidate.change.path === tab.path && candidate.scope === tab.scope
      );
      if (entry) openDiff(entry, queue);
      else setTabs((current) => closeReviewTab(current, reviewTabKey(tab)));
    } catch {
      // The existing changes surface owns retry/error presentation.
    }
  }, [workspace?.sessionId]);
  const activeTabKey = target.kind === "source"
    ? reviewTabKey({ ...target, pinned: false })
    : target.kind === "diff"
    ? reviewTabKey({ ...target, pinned: false })
    : undefined;
  const closeTab = (key: string): void => {
    const next = closeReviewTab(tabs, key);
    setTabs(next);
    if (activeTabKey !== key) return;
    const fallback = next.at(-1);
    if (fallback) void activateTab(fallback);
    else if (mode === "code") setSourceTarget(undefined);
    else setDiffTarget(undefined);
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
        mode === "code"
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
            <ReviewChanges
              key={`${workspace?.sessionId ?? "none"}:${dataRevision}`}
              sessionId={workspace?.sessionId}
              onOpenDiff={openDiff}
              reviewed={new Set(Object.keys(reviewProgress))}
              onRevision={adoptManifestRevision}
              drawer
              onClose={() => setCloseRequest((value) => value + 1)}
              refreshToken={dataRevision}
            />
          )
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
              onClick={() => {
                if (mode === "code") setSourceTarget(undefined);
                else setDiffTarget(undefined);
              }}
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
            boxShadow: (theme) =>
              theme.palette.mode === "dark"
                ? "0 -1px 24px rgba(0,0,0,0.5)"
                : "0 -1px 24px rgba(0,0,0,0.07)",
          }}
        >
          <ReviewTabStrip
            tabs={tabs}
            activeKey={activeTabKey}
            onActivate={(tab) => void activateTab(tab)}
            onClose={closeTab}
            onCloseOthers={(key) =>
              setTabs((current) => closeOtherReviewTabs(current, key))}
            onTogglePin={(key) =>
              setTabs((current) => toggleReviewTabPin(current, key))}
          />
          <Box
            component="nav"
            aria-label="Code Review controls"
            sx={{
              pb: "max(calc(env(safe-area-inset-bottom) - 18px), 12px)",
              pl: "env(safe-area-inset-left, 0px)",
              pr: "env(safe-area-inset-right, 0px)",
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
            {target.kind === "source" && isMarkdownReviewPath(target.path) && (
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
            <IconButton
              aria-label={`${mode === "git"
                ? "Disable Git review"
                : "Enable Git review"}${
                changeCount > 0
                  ? `, ${changeCount} changed ${changeCount === 1 ? "file" : "files"}`
                  : ""
              }`}
              aria-pressed={mode === "git"}
              color={mode === "git" ? "primary" : "default"}
              onClick={() =>
                setMode((current) => current === "git" ? "code" : "git")}
            >
              <Badge
                badgeContent={changeCount}
                max={99}
                color="primary"
                invisible={changeCount === 0}
              >
                <DifferenceOutlined />
              </Badge>
            </IconButton>
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
        </Box>
      </Stack>
    </ReviewDrawerShell>
  );
}
