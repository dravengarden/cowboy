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
import { useCallback, useEffect, useRef, useState } from "react";

interface FileTreeEntry {
  name: string;
  path: string;
  kind: "directory" | "file";
}

interface DirectoryPage {
  entries: FileTreeEntry[];
  truncated: boolean;
}

const directoryCache = new Map<string, DirectoryPage>();

function cacheKey(sessionId: string, path: string): string {
  return `${sessionId}\0${path}`;
}

function DirectoryRows({
  sessionId,
  entries,
  onOpenFile,
  depth = 0,
}: {
  sessionId: string;
  entries: FileTreeEntry[];
  onOpenFile: (path: string) => void;
  depth?: number;
}): React.JSX.Element {
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [pages, setPages] = useState<ReadonlyMap<string, DirectoryPage>>(
    new Map(),
  );
  const [loading, setLoading] = useState<ReadonlySet<string>>(new Set());
  const [failed, setFailed] = useState<ReadonlySet<string>>(new Set());
  const controllers = useRef(new Map<string, AbortController>());

  useEffect(() => () => {
    controllers.current.forEach((controller) => controller.abort());
  }, []);

  const loadDirectory = useCallback(async (path: string): Promise<void> => {
    const key = cacheKey(sessionId, path);
    const cached = directoryCache.get(key);
    if (cached) {
      setPages((current) => new Map(current).set(path, cached));
      return;
    }
    controllers.current.get(path)?.abort();
    const controller = new AbortController();
    controllers.current.set(path, controller);
    setLoading((current) => new Set(current).add(path));
    setFailed((current) => {
      const next = new Set(current);
      next.delete(path);
      return next;
    });
    try {
      const query = new URLSearchParams({ path });
      const response = await fetch(
        `/api/code/sessions/${encodeURIComponent(sessionId)}/tree?${query}`,
        { signal: controller.signal },
      );
      if (!response.ok) throw new Error(`file tree: ${response.status}`);
      const body = (await response.json()) as {
        apiVersion?: number;
        entries?: FileTreeEntry[];
        truncated?: boolean;
      };
      if (body.apiVersion !== 1) throw new Error("Unsupported Code API version");
      const page = {
        entries: body.entries ?? [],
        truncated: body.truncated ?? false,
      };
      directoryCache.set(key, page);
      setPages((current) => new Map(current).set(path, page));
    } catch (error) {
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        setFailed((current) => new Set(current).add(path));
      }
    } finally {
      controllers.current.delete(path);
      setLoading((current) => {
        const next = new Set(current);
        next.delete(path);
        return next;
      });
    }
  }, [sessionId]);

  return (
    <>
      {entries.map((entry) => {
        const isDirectory = entry.kind === "directory";
        const open = expanded.has(entry.path);
        const page = pages.get(entry.path);
        const isLoading = loading.has(entry.path);
        const hasFailed = failed.has(entry.path);
        return (
          <Box key={entry.path}>
            <ListItemButton
              aria-expanded={isDirectory ? open : undefined}
              onClick={() => {
                if (!isDirectory) {
                  onOpenFile(entry.path);
                  return;
                }
                setExpanded((current) => {
                  const next = new Set(current);
                  if (next.has(entry.path)) {
                    next.delete(entry.path);
                    controllers.current.get(entry.path)?.abort();
                  } else {
                    next.add(entry.path);
                    if (!page) void loadDirectory(entry.path);
                  }
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
              {isDirectory && (
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
                {isDirectory
                  ? <FolderOutlined fontSize="small" />
                  : <DescriptionOutlined fontSize="small" />}
              </ListItemIcon>
              <ListItemText
                primary={entry.name}
                primaryTypographyProps={{
                  noWrap: true,
                  fontFamily: isDirectory
                    ? undefined
                    : "var(--cowboy-font-mono)",
                  fontSize: 14,
                }}
              />
              {isLoading && <CircularProgress size={16} />}
            </ListItemButton>
            {isDirectory && open && hasFailed && (
              <Alert
                severity="error"
                sx={{ ml: 2 + depth * 2, py: 0 }}
                action={
                  <IconButton
                    size="small"
                    aria-label={`Retry ${entry.name}`}
                    onClick={() => void loadDirectory(entry.path)}
                  >
                    <Refresh fontSize="small" />
                  </IconButton>
                }
              >
                Could not load folder
              </Alert>
            )}
            {isDirectory && open && page && (
              <>
                <DirectoryRows
                  sessionId={sessionId}
                  entries={page.entries}
                  onOpenFile={onOpenFile}
                  depth={depth + 1}
                />
                {page.truncated && (
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ display: "block", pl: 4 + depth * 2, py: 0.5 }}
                  >
                    Folder limited to 200 entries
                  </Typography>
                )}
              </>
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
  onOpenFile,
}: {
  sessionId: string | undefined;
  cwd: string | undefined;
  onOpenFile: (path: string) => void;
}): React.JSX.Element {
  const [root, setRoot] = useState<DirectoryPage>({
    entries: [],
    truncated: false,
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [revision, setRevision] = useState(0);
  const controllerRef = useRef<AbortController | null>(null);

  const load = useCallback(async (refresh = false): Promise<void> => {
    if (!sessionId) {
      setRoot({ entries: [], truncated: false });
      return;
    }
    const key = cacheKey(sessionId, "");
    const cached = directoryCache.get(key);
    if (cached && !refresh) {
      setRoot(cached);
      return;
    }
    if (refresh) {
      for (const cacheKey of directoryCache.keys()) {
        if (cacheKey.startsWith(`${sessionId}\0`)) {
          directoryCache.delete(cacheKey);
        }
      }
      setRevision((current) => current + 1);
    }
    setLoading(true);
    setError(false);
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    try {
      const response = await fetch(
        `/api/code/sessions/${encodeURIComponent(sessionId)}/tree`,
        { signal: controller.signal },
      );
      if (!response.ok) throw new Error(`file tree: ${response.status}`);
      const body = (await response.json()) as {
        apiVersion?: number;
        entries?: FileTreeEntry[];
        truncated?: boolean;
      };
      if (body.apiVersion !== 1) throw new Error("Unsupported Code API version");
      const page = {
        entries: body.entries ?? [],
        truncated: body.truncated ?? false,
      };
      directoryCache.set(key, page);
      setRoot(page);
    } catch (error) {
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        setError(true);
      }
    } finally {
      if (controllerRef.current === controller) {
        controllerRef.current = null;
      }
      setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    void load();
    return () => controllerRef.current?.abort();
  }, [load]);

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
        <IconButton
          aria-label="Refresh worktree"
          onClick={() => void load(true)}
        >
          <Refresh />
        </IconButton>
      </Stack>
      {root.truncated && (
        <Alert severity="info" sx={{ mx: 1.5, mb: 1, py: 0 }}>
          Root folder limited to 200 entries
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
              {sessionId && (
                <DirectoryRows
                  key={`${sessionId}:${revision}`}
                  sessionId={sessionId}
                  entries={root.entries}
                  onOpenFile={onOpenFile}
                />
              )}
            </List>
          )}
      </Box>
    </Stack>
  );
}
