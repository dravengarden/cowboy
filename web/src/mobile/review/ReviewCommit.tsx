import {
  CommitOutlined,
  DescriptionOutlined,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Chip,
  CircularProgress,
  List,
  ListItemButton,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import {
  fetchGitCommit,
  fetchGitCommitDiff,
  type GitCommitDetail,
  type GitCommitSummary,
} from "./codeApi";

function shortOid(oid: string): string {
  return oid.slice(0, 8);
}

function commitBody(message: string, subject: string): string {
  const normalized = message.replaceAll("\r\n", "\n").trim();
  const [firstLine = "", ...remainingLines] = normalized.split("\n");
  if (firstLine.trim() !== subject.trim()) return normalized;
  return remainingLines.join("\n").trim();
}

function CommitPatch({
  sessionId,
  oid,
  path,
}: {
  sessionId: string;
  oid: string;
  path: string;
}): React.JSX.Element {
  const [patch, setPatch] = useState<
    Awaited<ReturnType<typeof fetchGitCommitDiff>>
  >();
  const [error, setError] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    setPatch(undefined);
    setError(false);
    void fetchGitCommitDiff(sessionId, oid, path, controller.signal)
      .then(setPatch)
      .catch((reason) => {
        if (!(reason instanceof DOMException && reason.name === "AbortError")) {
          setError(true);
        }
      });
    return () => controller.abort();
  }, [oid, path, sessionId]);

  const allLines = patch?.text.split("\n") ?? [];
  const lines = allLines.slice(0, 5_000);
  return (
    <Stack component="main" sx={{ height: 1, minHeight: 0 }}>
      <Box
        data-review-commit-patch
        sx={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          fontFamily: "var(--cowboy-font-mono)",
          fontSize: "0.72rem",
          lineHeight: 1.55,
        }}
      >
        {error
          ? <Alert severity="error">Couldn’t load this patch</Alert>
          : !patch
          ? (
            <Box sx={{ display: "grid", placeItems: "center", pt: 8 }}>
              <CircularProgress size={24} />
            </Box>
          )
          : lines.map((line, index) => (
            <Box
              key={index}
              component="div"
              sx={{
                minWidth: "max-content",
                px: 1.5,
                whiteSpace: "pre",
                color: line.startsWith("+") && !line.startsWith("+++")
                  ? "success.main"
                  : line.startsWith("-") && !line.startsWith("---")
                  ? "error.main"
                  : line.startsWith("@@")
                  ? "primary.main"
                  : "text.secondary",
              }}
            >
              {line || " "}
            </Box>
          ))}
        {patch && (patch.truncated || allLines.length > lines.length) && (
          <Alert severity="info">Large patch preview truncated</Alert>
        )}
      </Box>
    </Stack>
  );
}

export function ReviewCommit({
  sessionId,
  commit,
  selectedPath,
  onSelectPath,
  onFiles,
}: {
  sessionId: string;
  commit: GitCommitSummary;
  selectedPath?: string | undefined;
  onSelectPath: (path: string | undefined) => void;
  onFiles?: (paths: readonly string[]) => void;
}): React.JSX.Element {
  const [detail, setDetail] = useState<GitCommitDetail>();
  const [error, setError] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    setDetail(undefined);
    setError(false);
    void fetchGitCommit(sessionId, commit.oid, controller.signal)
      .then(setDetail)
      .catch((reason) => {
        if (!(reason instanceof DOMException && reason.name === "AbortError")) {
          setError(true);
        }
      });
    return () => controller.abort();
  }, [commit.oid, sessionId]);
  useEffect(() => {
    onFiles?.(detail?.files.map((file) => file.path) ?? []);
  }, [detail, onFiles]);
  useEffect(() => () => onFiles?.([]), [onFiles]);

  if (selectedPath) {
    return (
      <CommitPatch
        sessionId={sessionId}
        oid={commit.oid}
        path={selectedPath}
      />
    );
  }

  const body = detail ? commitBody(detail.message, commit.subject) : "";
  return (
    <Box
      component="main"
      data-review-commit
      data-mobile-overflow-layer="true"
      sx={{
        flex: 1,
        minHeight: 0,
        px: 2,
        py: 2,
        ...mobileNativeYScrollSx,
      }}
    >
      {error
        ? <Alert severity="error">Couldn’t load this commit</Alert>
        : !detail
        ? (
          <Box sx={{ display: "grid", placeItems: "center", pt: 8 }}>
            <CircularProgress size={24} />
          </Box>
        )
        : (
          <Stack spacing={2.5}>
            <Box
              sx={{
                p: 2,
                border: 1,
                borderColor: "divider",
                borderRadius: 3,
                bgcolor: "action.hover",
              }}
            >
              <Stack direction="row" spacing={1.25} alignItems="flex-start">
                <CommitOutlined color="primary" sx={{ mt: 0.25 }} />
                <Box sx={{ minWidth: 0, flex: 1 }}>
                  <Typography variant="subtitle1" fontWeight={700}>
                    {commit.subject}
                  </Typography>
                  {body && (
                    <Typography
                      variant="body2"
                      color="text.secondary"
                      sx={{
                        mt: 1,
                        whiteSpace: "pre-wrap",
                        overflowWrap: "anywhere",
                      }}
                    >
                      {body}
                    </Typography>
                  )}
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ mt: 1.5, display: "block" }}
                  >
                    {detail.author} ·{" "}
                    {new Date(detail.authoredAt).toLocaleString()}
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    {shortOid(detail.oid)} ·{" "}
                    {detail.parents.length > 1 ? "Merge" : "Commit"}
                  </Typography>
                </Box>
              </Stack>
            </Box>

            <Box>
              <Stack
                direction="row"
                alignItems="center"
                spacing={1}
                sx={{ mb: 1 }}
              >
                <DescriptionOutlined color="action" fontSize="small" />
                <Typography variant="overline" color="text.secondary">
                  {detail.files.length} changed files
                </Typography>
              </Stack>
              <List
                disablePadding
                sx={{
                  border: 1,
                  borderColor: "divider",
                  borderRadius: 3,
                  overflow: "hidden",
                }}
              >
                {detail.files.map((file) => (
                  <ListItemButton
                    key={`${file.oldPath ?? ""}:${file.path}`}
                    onClick={() => onSelectPath(file.path)}
                    sx={{
                      minHeight: 56,
                      gap: 1,
                      borderBottom: 1,
                      borderColor: "divider",
                      "&:last-child": { borderBottom: 0 },
                    }}
                  >
                    <Chip
                      size="small"
                      label={file.status.slice(0, 1).toUpperCase()}
                      sx={{ minWidth: 28 }}
                    />
                    <ListItemText
                      primary={file.path}
                      secondary={file.oldPath
                        ? `from ${file.oldPath}`
                        : undefined}
                      primaryTypographyProps={{
                        variant: "body2",
                        sx: { overflowWrap: "anywhere" },
                      }}
                    />
                  </ListItemButton>
                ))}
              </List>
              {detail.filesTruncated && (
                <Alert severity="info" sx={{ mt: 1 }}>
                  Showing the first 1,000 files
                </Alert>
              )}
            </Box>
          </Stack>
        )}
    </Box>
  );
}
