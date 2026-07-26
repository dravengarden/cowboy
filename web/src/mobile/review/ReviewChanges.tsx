import {
  Add,
  CallSplit,
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
import { useCallback, useEffect, useState } from "react";
import {
  type CodeChange,
  type CodeChangeStatus,
  fetchCodeChanges,
} from "./codeApi";

const statusLabel: Record<CodeChangeStatus, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "!",
};

function ChangeIcon({ status }: { status: CodeChangeStatus }): React.JSX.Element {
  if (status === "added" || status === "untracked") return <Add />;
  if (status === "deleted") return <DeleteOutline />;
  if (status === "renamed") return <CallSplit />;
  if (status === "conflicted") return <ErrorOutline />;
  return <DescriptionOutlined />;
}

export function ReviewChanges({
  sessionId,
  onOpenDiff,
}: {
  sessionId: string | undefined;
  onOpenDiff: (path: string) => void;
}): React.JSX.Element {
  const [changes, setChanges] = useState<CodeChange[]>([]);
  const [head, setHead] = useState<string>();
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

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
          <Typography variant="subtitle1" fontWeight={700}>Changes</Typography>
          <Typography variant="caption" color="text.secondary">
            {head ? `HEAD ${head}` : "Working tree"}
          </Typography>
        </Box>
        <Chip size="small" label={changes.length} />
        <IconButton aria-label="Refresh changes" onClick={() => void load()}>
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
          ? (
            <Alert severity="error">Git changes are unavailable</Alert>
          )
          : changes.length === 0
          ? (
            <Stack alignItems="center" spacing={1} sx={{ pt: 10 }}>
              <DescriptionOutlined color="disabled" />
              <Typography color="text.secondary">Working tree is clean</Typography>
            </Stack>
          )
          : (
            <List disablePadding>
              {changes.map((change) => (
                <ListItemButton
                  key={`${change.status}:${change.path}`}
                  onClick={() => onOpenDiff(change.path)}
                  sx={{ minHeight: 52, borderRadius: 2 }}
                >
                  <ListItemIcon sx={{ minWidth: 36, color: "text.secondary" }}>
                    <ChangeIcon status={change.status} />
                  </ListItemIcon>
                  <ListItemText
                    primary={change.path.split("/").pop()}
                    secondary={change.path.includes("/")
                      ? change.path.slice(0, change.path.lastIndexOf("/"))
                      : change.oldPath
                      ? `from ${change.oldPath}`
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
                    label={statusLabel[change.status]}
                    color={change.status === "conflicted" ? "error" : "default"}
                    sx={{ minWidth: 32 }}
                  />
                </ListItemButton>
              ))}
            </List>
          )}
      </Box>
    </Stack>
  );
}
