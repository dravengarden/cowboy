import {
  Close,
  ChevronRight,
  DescriptionOutlined,
  FolderOutlined,
  MyLocation,
  Refresh,
  Search,
  UnfoldLess,
  UnfoldMore,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  CircularProgress,
  IconButton,
  InputAdornment,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useRef, useState } from "react";
import { MobileSheetActionGroup } from "../../_shell";
import {
  type CodeTreeEntry as FileTreeEntry,
  type CodeTreePage,
  fetchCodeSearch,
  fetchCodeTree,
} from "./codeApi";

type DirectoryPage = CodeTreePage & { cachedAt: number };

const directoryCache = new Map<string, DirectoryPage>();
const MEMORY_FRESH_MS = 15_000;

function cacheKey(sessionId: string, path: string): string {
  return `${sessionId}\0${path}`;
}

function DirectoryRows({
  entries,
  onOpenFile,
  expanded,
  pages,
  loading,
  failed,
  onToggleDirectory,
  currentPath,
  depth = 0,
}: {
  entries: FileTreeEntry[];
  onOpenFile: (path: string) => void;
  expanded: ReadonlySet<string>;
  pages: ReadonlyMap<string, DirectoryPage>;
  loading: ReadonlySet<string>;
  failed: ReadonlySet<string>;
  onToggleDirectory: (path: string) => void;
  currentPath: string | undefined;
  depth?: number;
}): React.JSX.Element {
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
              data-code-tree-path={entry.path}
              selected={!isDirectory && entry.path === currentPath}
              aria-expanded={isDirectory ? open : undefined}
              onClick={() => {
                if (!isDirectory) {
                  onOpenFile(entry.path);
                  return;
                }
                onToggleDirectory(entry.path);
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
                    onClick={() => onToggleDirectory(entry.path)}
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
                  entries={page.entries}
                  onOpenFile={onOpenFile}
                  expanded={expanded}
                  pages={pages}
                  loading={loading}
                  failed={failed}
                  onToggleDirectory={onToggleDirectory}
                  currentPath={currentPath}
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
  currentPath,
  onClose,
  refreshToken,
}: {
  sessionId: string | undefined;
  cwd: string | undefined;
  onOpenFile: (path: string) => void;
  currentPath: string | undefined;
  onClose: () => void;
  refreshToken: number;
}): React.JSX.Element {
  const [root, setRoot] = useState<DirectoryPage>({
    apiVersion: 1,
    path: "",
    revision: "",
    entries: [],
    truncated: false,
    cachedAt: 0,
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [revision, setRevision] = useState(0);
  const [query, setQuery] = useState("");
  const [searchResults, setSearchResults] = useState<string[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchFailed, setSearchFailed] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [pages, setPages] = useState<ReadonlyMap<string, DirectoryPage>>(
    new Map(),
  );
  const [directoryLoading, setDirectoryLoading] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [directoryFailed, setDirectoryFailed] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const controllerRef = useRef<AbortController | null>(null);
  const searchControllerRef = useRef<AbortController | null>(null);
  const directoryControllers = useRef(new Map<string, AbortController>());
  const treeScrollerRef = useRef<HTMLDivElement>(null);
  const previousRefreshToken = useRef(refreshToken);

  const load = useCallback(async (refresh = false): Promise<void> => {
    if (!sessionId) {
      setRoot({
        apiVersion: 1,
        path: "",
        revision: "",
        entries: [],
        truncated: false,
        cachedAt: 0,
      });
      return;
    }
    const key = cacheKey(sessionId, "");
    const cached = directoryCache.get(key);
    if (cached && !refresh) {
      setRoot(cached);
      if (Date.now() - cached.cachedAt <= MEMORY_FRESH_MS) return;
    }
    if (refresh) {
      for (const cacheKey of directoryCache.keys()) {
        if (cacheKey.startsWith(`${sessionId}\0`)) {
          directoryCache.delete(cacheKey);
        }
      }
      setRevision((current) => current + 1);
    }
    setLoading(!cached);
    setError(false);
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    try {
      const page = {
        ...(await fetchCodeTree(sessionId, "", controller.signal, refresh)),
        cachedAt: Date.now(),
      };
      directoryCache.set(key, page);
      setRoot(page);
    } catch (error) {
      if (
        !cached &&
        !(error instanceof DOMException && error.name === "AbortError")
      ) {
        setError(true);
      }
    } finally {
      if (controllerRef.current === controller) {
        controllerRef.current = null;
        setLoading(false);
      }
    }
  }, [sessionId]);

  const loadDirectory = useCallback(async (path: string): Promise<void> => {
    if (!sessionId) return;
    const key = cacheKey(sessionId, path);
    const cached = directoryCache.get(key);
    if (cached) {
      setPages((current) => new Map(current).set(path, cached));
      if (Date.now() - cached.cachedAt <= MEMORY_FRESH_MS) return;
    }
    directoryControllers.current.get(path)?.abort();
    const controller = new AbortController();
    directoryControllers.current.set(path, controller);
    if (!cached) {
      setDirectoryLoading((current) => new Set(current).add(path));
    }
    setDirectoryFailed((current) => {
      const next = new Set(current);
      next.delete(path);
      return next;
    });
    try {
      const page = {
        ...(await fetchCodeTree(sessionId, path, controller.signal)),
        cachedAt: Date.now(),
      };
      directoryCache.set(key, page);
      setPages((current) => new Map(current).set(path, page));
    } catch (error) {
      if (
        !cached &&
        !(error instanceof DOMException && error.name === "AbortError")
      ) {
        setDirectoryFailed((current) => new Set(current).add(path));
      }
    } finally {
      if (directoryControllers.current.get(path) === controller) {
        directoryControllers.current.delete(path);
        setDirectoryLoading((current) => {
          const next = new Set(current);
          next.delete(path);
          return next;
        });
      }
    }
  }, [sessionId]);

  const toggleDirectory = useCallback((path: string): void => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
        directoryControllers.current.get(path)?.abort();
      } else {
        next.add(path);
        void loadDirectory(path);
      }
      return next;
    });
  }, [loadDirectory]);

  const rootDirectories = root.entries
    .filter((entry) => entry.kind === "directory")
    .map((entry) => entry.path);
  const rootExpanded = rootDirectories.length > 0 &&
    rootDirectories.every((path) => expanded.has(path));
  const toggleRootDirectories = (): void => {
    if (rootExpanded) {
      setExpanded(new Set());
      return;
    }
    setExpanded((current) => new Set([...current, ...rootDirectories]));
    for (const path of rootDirectories) void loadDirectory(path);
  };

  const revealCurrentFile = async (): Promise<void> => {
    if (!currentPath || !sessionId) return;
    setQuery("");
    const parts = currentPath.split("/");
    const ancestors = parts.slice(0, -1).map((_, index) =>
      parts.slice(0, index + 1).join("/")
    );
    setExpanded((current) => new Set([...current, ...ancestors]));
    await Promise.all(ancestors.map(loadDirectory));
    requestAnimationFrame(() => {
      const escaped = CSS.escape(currentPath);
      treeScrollerRef.current
        ?.querySelector<HTMLElement>(`[data-code-tree-path="${escaped}"]`)
        ?.scrollIntoView({ behavior: "smooth", block: "center" });
    });
  };

  useEffect(() => {
    void load();
    return () => {
      controllerRef.current?.abort();
      searchControllerRef.current?.abort();
      directoryControllers.current.forEach((controller) => controller.abort());
    };
  }, [load]);

  useEffect(() => {
    if (previousRefreshToken.current === refreshToken) return;
    previousRefreshToken.current = refreshToken;
    void load(true);
  }, [load, refreshToken]);

  useEffect(() => {
    searchControllerRef.current?.abort();
    const trimmed = query.trim();
    if (!sessionId || !trimmed) {
      setSearchResults([]);
      setSearching(false);
      setSearchFailed(false);
      return undefined;
    }
    const controller = new AbortController();
    searchControllerRef.current = controller;
    setSearching(true);
    setSearchFailed(false);
    const timer = globalThis.setTimeout(() => {
      void fetchCodeSearch(sessionId, trimmed, controller.signal)
        .then((result) => setSearchResults(result.files))
        .catch((error) => {
          if (!(error instanceof DOMException && error.name === "AbortError")) {
            setSearchFailed(true);
          }
        })
        .finally(() => {
          if (searchControllerRef.current === controller) {
            searchControllerRef.current = null;
            setSearching(false);
          }
        });
    }, 180);
    return () => {
      globalThis.clearTimeout(timer);
      controller.abort();
    };
  }, [query, sessionId]);

  return (
    <Stack sx={{ position: "relative", height: "100%", minHeight: 0 }}>
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
      </Stack>
      <TextField
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Find a file"
        aria-label="Find a file"
        size="small"
        fullWidth
        slotProps={{
          input: {
            startAdornment: (
              <InputAdornment position="start">
                <Search fontSize="small" />
              </InputAdornment>
            ),
            endAdornment: query
              ? (
                <InputAdornment position="end">
                  <IconButton
                    size="small"
                    aria-label="Clear file search"
                    onClick={() => setQuery("")}
                  >
                    <Close fontSize="small" />
                  </IconButton>
                </InputAdornment>
              )
              : undefined,
          },
        }}
        sx={{ px: 1.5, pb: 1.25 }}
      />
      {root.truncated && (
        <Alert severity="info" sx={{ mx: 1.5, mb: 1, py: 0 }}>
          Root folder limited to 200 entries
        </Alert>
      )}
      <Box
        ref={treeScrollerRef}
        sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.75, pb: 12 }}
      >
        {query.trim()
          ? searching
            ? (
              <Box sx={{ display: "grid", placeItems: "center", pt: 6 }}>
                <CircularProgress size={24} />
              </Box>
            )
            : searchFailed
            ? <Alert severity="error">File search is unavailable</Alert>
            : searchResults.length === 0
            ? (
              <Typography
                color="text.secondary"
                textAlign="center"
                sx={{ pt: 6 }}
              >
                No matching files
              </Typography>
            )
            : (
              <List disablePadding>
                {searchResults.map((path) => {
                  const split = path.lastIndexOf("/");
                  return (
                    <ListItemButton
                      key={path}
                      onClick={() => onOpenFile(path)}
                      sx={{ minHeight: 48, borderRadius: 2 }}
                    >
                      <ListItemIcon
                        sx={{ minWidth: 34, color: "text.secondary" }}
                      >
                        <DescriptionOutlined fontSize="small" />
                      </ListItemIcon>
                      <ListItemText
                        primary={split >= 0 ? path.slice(split + 1) : path}
                        secondary={split >= 0 ? path.slice(0, split) : undefined}
                        primaryTypographyProps={{
                          noWrap: true,
                          fontFamily: "var(--cowboy-font-mono)",
                          fontSize: 14,
                        }}
                        secondaryTypographyProps={{ noWrap: true }}
                      />
                    </ListItemButton>
                  );
                })}
              </List>
            )
          : loading
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
                  entries={root.entries}
                  onOpenFile={onOpenFile}
                  expanded={expanded}
                  pages={pages}
                  loading={directoryLoading}
                  failed={directoryFailed}
                  onToggleDirectory={toggleDirectory}
                  currentPath={currentPath}
                />
              )}
            </List>
          )}
      </Box>
      <Box
        sx={{
          position: "absolute",
          zIndex: 2,
          left: 0,
          right: 0,
          bottom: "max(env(safe-area-inset-bottom, 0px), 12px)",
          px: 2,
          pointerEvents: "none",
        }}
      >
        <MobileSheetActionGroup
          actions={[
            {
              key: "close",
              label: "Close worktree",
              icon: <Close fontSize="small" />,
              onPress: onClose,
            },
            {
              key: "expand",
              label: rootExpanded ? "Collapse folders" : "Expand folders",
              icon: rootExpanded
                ? <UnfoldLess fontSize="small" />
                : <UnfoldMore fontSize="small" />,
              onPress: toggleRootDirectories,
              disabled: rootDirectories.length === 0,
            },
            {
              key: "locate",
              label: "Reveal current file",
              icon: <MyLocation fontSize="small" />,
              onPress: () => void revealCurrentFile(),
              disabled: !currentPath,
            },
          ]}
        />
      </Box>
    </Stack>
  );
}
