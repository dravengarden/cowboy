import {
  ArrowBack,
  ChevronLeft,
  ChevronRight,
  FolderOpenOutlined,
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
import { fetchCodeDiff, fetchCodeFile } from "./codeApi";
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

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError(false);
    setText("");
    setCounts(undefined);
    const request = target.kind === "diff"
      ? fetchCodeDiff(
        sessionId,
        target.path,
        settings.contextLines,
        settings.showWhitespaceChanges,
        target.scope,
        controller.signal,
      )
      : fetchCodeFile(sessionId, target.path, controller.signal);
    void request.then((result) => {
      setText(result.text);
      setTruncated(result.truncated);
      if ("added" in result) {
        setCounts({ added: result.added, removed: result.removed });
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

  useEffect(() => setTarget({ kind: "changes" }), [workspace?.sessionId]);

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
            />
          )
          : <DocumentView sessionId={workspace.sessionId} target={target} />}
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
