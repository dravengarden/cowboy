import {
  Add,
  CallSplit,
  CheckCircleOutline,
  DeleteOutline,
  DescriptionOutlined,
  ErrorOutline,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Chip,
  CircularProgress,
  IconButton,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import { type CodeChangeStatus, fetchCodeChanges } from "./codeApi";
import {
  type GitReviewEntry,
  groupGitChanges,
  reviewQueue,
} from "./gitReviewModel";
import { invalidateDiffCache } from "./diffCache";
import { reviewEntryKey } from "./diffNavigationModel";

const statusLabel: Record<CodeChangeStatus, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "!",
};

function ChangeIcon(
  { status }: { status: CodeChangeStatus },
): React.JSX.Element {
  if (status === "added" || status === "untracked") return <Add />;
  if (status === "deleted") return <DeleteOutline />;
  if (status === "renamed") return <CallSplit />;
  if (status === "conflicted") return <ErrorOutline />;
  return <DescriptionOutlined />;
}

export function ReviewChanges({
  sessionId,
  onOpenDiff,
  reviewed,
  onRefresh,
}: {
  sessionId: string | undefined;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  reviewed: ReadonlySet<string>;
  onRefresh: () => void;
}): React.JSX.Element {
  const [changes, setChanges] = useState<
    Awaited<
      ReturnType<typeof fetchCodeChanges>
    >["changes"]
  >([]);
  const [head, setHead] = useState<string>();
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const sections = useMemo(() => groupGitChanges(changes), [changes]);
  const queue = useMemo(() => reviewQueue(sections), [sections]);

  const load = useCallback(async (signal?: AbortSignal): Promise<void> => {
    if (!sessionId) {
      setChanges([]);
      return;
    }
    setLoading(true);
    setError(false);
    try {
      const result = await fetchCodeChanges(sessionId, signal);
      setChanges(result.changes);
      setHead(result.head);
      setTruncated(result.truncated);
    } catch (reason) {
      if (!(reason instanceof DOMException && reason.name === "AbortError")) {
        setError(true);
      }
    } finally {
      if (!signal?.aborted) setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load]);

  return (
    <Stack sx={{ height: "100%", minHeight: 0 }}>
      <Stack direction="row" alignItems="center" sx={{ px: 2, py: 1 }}>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="subtitle1" fontWeight={700}>
            Git review
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {head ? `HEAD ${head}` : "Working tree"}
          </Typography>
        </Box>
        <Chip
          size="small"
          label={`${
            queue.filter((entry) =>
              reviewed.has(reviewEntryKey(entry.change.path, entry.scope))
            ).length
          } / ${queue.length}`}
        />
        <IconButton
          aria-label="Refresh changes"
          onClick={() => {
            invalidateDiffCache(sessionId);
            onRefresh();
            void load();
          }}
        >
          <Refresh />
        </IconButton>
      </Stack>
      {truncated && (
        <Alert severity="info" sx={{ mx: 1.5, mb: 1, py: 0 }}>
          Showing the first 1,000 changes
        </Alert>
      )}
      <Box sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.75, pb: 8 }}>
        {loading
          ? (
            <Box sx={{ display: "grid", placeItems: "center", pt: 8 }}>
              <CircularProgress size={24} />
            </Box>
          )
          : error
          ? <Alert severity="error">Git changes are unavailable</Alert>
          : changes.length === 0
          ? (
            <Stack alignItems="center" spacing={1} sx={{ pt: 10 }}>
              <DescriptionOutlined color="disabled" />
              <Typography color="text.secondary">
                Working tree is clean
              </Typography>
            </Stack>
          )
          : (
            <Stack spacing={1.25}>
              {sections.map((section) => (
                <Box component="section" key={section.kind}>
                  <Stack
                    direction="row"
                    alignItems="center"
                    sx={{ px: 1.25, minHeight: 36 }}
                  >
                    <Typography
                      variant="overline"
                      color={section.kind === "conflicts"
                        ? "error.main"
                        : "text.secondary"}
                      sx={{ flex: 1, letterSpacing: "0.08em" }}
                    >
                      {section.label}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      {section.entries.length}
                    </Typography>
                  </Stack>
                  <List
                    disablePadding
                    sx={{
                      border: 1,
                      borderColor: "divider",
                      borderRadius: 2.5,
                      overflow: "hidden",
                    }}
                  >
                    {section.entries.map((entry, index) => (
                      <ListItemButton
                        key={`${entry.scope}:${entry.change.path}`}
                        onClick={() => onOpenDiff(entry, queue)}
                        divider={index < section.entries.length - 1}
                        sx={{ minHeight: 56, px: 1.25 }}
                      >
                        <ListItemIcon
                          sx={{
                            minWidth: 36,
                            color: reviewed.has(
                                reviewEntryKey(
                                  entry.change.path,
                                  entry.scope,
                                ),
                              )
                              ? "success.main"
                              : "text.secondary",
                          }}
                        >
                          {reviewed.has(
                              reviewEntryKey(entry.change.path, entry.scope),
                            )
                            ? <CheckCircleOutline />
                            : <ChangeIcon status={entry.change.status} />}
                        </ListItemIcon>
                        <ListItemText
                          primary={entry.change.path.split("/").pop()}
                          secondary={entry.change.path.includes("/")
                            ? entry.change.path.slice(
                              0,
                              entry.change.path.lastIndexOf("/"),
                            )
                            : entry.change.oldPath
                            ? `from ${entry.change.oldPath}`
                            : undefined}
                          primaryTypographyProps={{
                            noWrap: true,
                            fontFamily: "var(--cowboy-font-mono)",
                            fontSize: 14,
                          }}
                          secondaryTypographyProps={{ noWrap: true }}
                        />
                        <Chip
                          size="small"
                          label={statusLabel[entry.change.status]}
                          color={entry.change.status === "conflicted"
                            ? "error"
                            : "default"}
                          sx={{ minWidth: 32 }}
                        />
                      </ListItemButton>
                    ))}
                  </List>
                </Box>
              ))}
            </Stack>
          )}
      </Box>
    </Stack>
  );
}
