import {
  ChevronRight,
  DescriptionOutlined,
  FolderOutlined,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
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
import { buildFileTree, type FileTreeNode } from "./fileTree";

function TreeRows({
  nodes,
  depth = 0,
}: {
  nodes: FileTreeNode[];
  depth?: number;
}): React.JSX.Element {
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  return (
    <>
      {nodes.map((node) => {
        const open = expanded.has(node.path);
        return (
          <Box key={node.path}>
            <ListItemButton
              aria-expanded={node.kind === "directory" ? open : undefined}
              onClick={() => {
                if (node.kind !== "directory") return;
                setExpanded((current) => {
                  const next = new Set(current);
                  if (next.has(node.path)) next.delete(node.path);
                  else next.add(node.path);
                  return next;
                });
              }}
              sx={{
                minHeight: 44,
                pl: 1.5 + depth * 2,
                pr: 1,
                borderRadius: 2,
              }}
            >
              {node.kind === "directory" && (
                <ChevronRight
                  sx={{
                    mr: 0.5,
                    fontSize: 18,
                    color: "text.secondary",
                    transform: open ? "rotate(90deg)" : "none",
                    transition: "transform 140ms ease",
                  }}
                />
              )}
              <ListItemIcon sx={{ minWidth: 30, color: "text.secondary" }}>
                {node.kind === "directory"
                  ? <FolderOutlined fontSize="small" />
                  : <DescriptionOutlined fontSize="small" />}
              </ListItemIcon>
              <ListItemText
                primary={node.name}
                primaryTypographyProps={{
                  noWrap: true,
                  fontFamily: node.kind === "file"
                    ? "var(--cowboy-font-mono)"
                    : undefined,
                  fontSize: 14,
                }}
              />
            </ListItemButton>
            {node.kind === "directory" && open && (
              <TreeRows nodes={node.children} depth={depth + 1} />
            )}
          </Box>
        );
      })}
    </>
  );
}

export function ReviewFileTree({
  sessionId,
  cwd,
}: {
  sessionId: string | undefined;
  cwd: string | undefined;
}): React.JSX.Element {
  const [files, setFiles] = useState<string[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    if (!sessionId) {
      setFiles([]);
      return;
    }
    setLoading(true);
    setError(false);
    try {
      const response = await fetch(
        `/api/sessions/${encodeURIComponent(sessionId)}/file-tree`,
      );
      if (!response.ok) throw new Error(`file tree: ${response.status}`);
      const body = (await response.json()) as {
        files?: string[];
        truncated?: boolean;
      };
      setFiles(body.files ?? []);
      setTruncated(body.truncated ?? false);
    } catch {
      setError(true);
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  const tree = useMemo(() => buildFileTree(files), [files]);
  return (
    <Stack sx={{ height: "100%", minHeight: 0 }}>
      <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        sx={{
          pt: "calc(env(safe-area-inset-top, 0px) + 18px)",
          px: 2,
          pb: 1.5,
        }}
      >
        <Box sx={{ minWidth: 0, flex: 1 }}>
          <Typography variant="subtitle1" fontWeight={700}>
            Worktree
          </Typography>
          <Typography variant="caption" color="text.secondary" noWrap>
            {cwd ?? "No active session"}
          </Typography>
        </Box>
        <IconButton aria-label="Refresh worktree" onClick={() => void load()}>
          <Refresh />
        </IconButton>
      </Stack>
      {truncated && (
        <Alert severity="info" sx={{ mx: 1.5, mb: 1, py: 0 }}>
          Showing the first 5,000 files
        </Alert>
      )}
      <Box sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.75, pb: 2 }}>
        {loading
          ? (
            <Box sx={{ display: "grid", placeItems: "center", pt: 6 }}>
              <CircularProgress size={24} />
            </Box>
          )
          : error
          ? (
            <Alert
              severity="error"
              action={
                <IconButton
                  size="small"
                  aria-label="Retry"
                  onClick={() => void load()}
                >
                  <Refresh fontSize="small" />
                </IconButton>
              }
            >
              Could not load the worktree
            </Alert>
          )
          : (
            <List disablePadding>
              <TreeRows nodes={tree} />
            </List>
          )}
      </Box>
    </Stack>
  );
}
