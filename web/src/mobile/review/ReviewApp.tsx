import {
  ArrowBack,
  CheckCircle,
  CheckCircleOutline,
  ChevronLeft,
  ChevronRight,
  FolderOpenOutlined,
  KeyboardArrowDown,
  KeyboardArrowUp,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Chip,
  CircularProgress,
  IconButton,
  Stack,
  Toolbar,
  Typography,
} from "@mui/material";
import { lazy, Suspense, useEffect, useState } from "react";
import { useActiveWorkspaceBinding } from "../../controlPlane";
import { fetchCodeFile } from "./codeApi";
import { loadCodeDiff } from "./diffCache";
import { diffHunkLines, reviewEntryKey } from "./diffNavigationModel";
import { ReviewChanges } from "./ReviewChanges";
import { ReviewDrawerShell } from "./ReviewDrawerShell";
import { ReviewFileTree } from "./ReviewFileTree";
import { ReviewSettings } from "./ReviewSettings";
import { useReviewSettings } from "./reviewSettings";
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
}: {
  sessionId: string;
  target: Exclude<ReviewTarget, { kind: "changes" }>;
}): React.JSX.Element {
  const settings = useReviewSettings();
  const [text, setText] = useState("");
  const [truncated, setTruncated] = useState(false);
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
      if ("added" in result) {
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
    return () => controller.abort();
  }, [
    sessionId,
    settings.contextLines,
    settings.showWhitespaceChanges,
    target.kind,
    target.path,
    target.kind === "diff" ? target.scope : undefined,
  ]);

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
      {(truncated || counts) && (
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
          {truncated && (
            <Chip
              size="small"
              color="warning"
              label="Preview limited to 2 MB"
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
            softWrap={settings.softWrap}
            revealLine={target.kind === "diff" ? hunks[hunkIndex] : undefined}
          />
        </Suspense>
      </Box>
    </Stack>
  );
}

export function ReviewApp({
  onDrawerOpenChange,
}: {
  onDrawerOpenChange: (open: boolean) => void;
}): React.JSX.Element {
  const workspace = useActiveWorkspaceBinding();
  const [target, setTarget] = useState<ReviewTarget>({ kind: "changes" });
  const [closeRequest, setCloseRequest] = useState(0);
  const [reviewed, setReviewed] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    setTarget({ kind: "changes" });
    setReviewed(new Set());
  }, [workspace?.sessionId]);

  const openSource = (path: string): void => {
    setTarget({ kind: "source", path });
    setCloseRequest((value) => value + 1);
  };
  const openDiff = (
    entry: GitReviewEntry,
    queue: GitReviewEntry[],
  ): void => {
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
  const toggleReviewed = (): void => {
    if (!targetReviewKey) return;
    setReviewed((current) => {
      const next = new Set(current);
      if (next.has(targetReviewKey)) next.delete(targetReviewKey);
      else next.add(targetReviewKey);
      return next;
    });
  };

  return (
    <ReviewDrawerShell
      onOpenChange={onDrawerOpenChange}
      closeRequest={closeRequest}
      drawer={
        <ReviewFileTree
          sessionId={workspace?.sessionId}
          cwd={workspace?.cwd}
          onOpenFile={openSource}
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
                aria-label={targetReviewKey && reviewed.has(targetReviewKey)
                  ? "Mark unreviewed"
                  : "Mark reviewed"}
                color={targetReviewKey && reviewed.has(targetReviewKey)
                  ? "success"
                  : "default"}
                onClick={toggleReviewed}
              >
                {targetReviewKey && reviewed.has(targetReviewKey)
                  ? <CheckCircle />
                  : <CheckCircleOutline />}
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
              sessionId={workspace.sessionId}
              onOpenDiff={openDiff}
              reviewed={reviewed}
              onRefresh={() => setReviewed(new Set())}
            />
          )
          : (
            <DocumentView
              sessionId={workspace.sessionId}
              target={target}
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
          </Toolbar>
        </Box>
      </Stack>
    </ReviewDrawerShell>
  );
}
