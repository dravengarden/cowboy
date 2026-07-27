import {
  Add,
  CallSplit,
  CheckCircleOutline,
  ChevronRight,
  DeleteOutline,
  DescriptionOutlined,
  ErrorOutline,
  ExpandMore,
  FolderOutlined,
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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MobileSheetDismiss } from "../../_shell";
import { type CodeChangeStatus, fetchCodeChanges } from "./codeApi";
import {
  type GitReviewEntry,
  groupGitChanges,
  limitGitSections,
  reviewQueue,
} from "./gitReviewModel";
import {
  buildGitChangeTree,
  type GitChangeTreeNode,
} from "./gitChangeTree";
import { reviewEntryKey } from "./diffNavigationModel";

const statusLabel: Record<CodeChangeStatus, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "!",
};

const REVIEW_WINDOW_SIZE = 80;

function ChangeIcon(
  { status }: { status: CodeChangeStatus },
): React.JSX.Element {
  if (status === "added" || status === "untracked") return <Add />;
  if (status === "deleted") return <DeleteOutline />;
  if (status === "renamed") return <CallSplit />;
  if (status === "conflicted") return <ErrorOutline />;
  return <DescriptionOutlined />;
}

function GitTreeRows({
  nodes,
  depth,
  collapsed,
  onToggle,
  onOpenDiff,
  queue,
  reviewed,
}: {
  nodes: GitChangeTreeNode[];
  depth: number;
  collapsed: ReadonlySet<string>;
  onToggle: (path: string) => void;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  queue: GitReviewEntry[];
  reviewed: ReadonlySet<string>;
}): React.JSX.Element {
  return (
    <>
      {nodes.map((node) => {
        if (node.kind === "directory") {
          const isCollapsed = collapsed.has(node.path);
          return (
            <Box key={`directory:${node.path}`}>
              <ListItemButton
                onClick={() => onToggle(node.path)}
                sx={{
                  minHeight: 46,
                  pl: 1 + depth * 2,
                  pr: 1.25,
                }}
              >
                <ListItemIcon sx={{ minWidth: 30, color: "text.secondary" }}>
                  {isCollapsed
                    ? <ChevronRight fontSize="small" />
                    : <ExpandMore fontSize="small" />}
                </ListItemIcon>
                <FolderOutlined
                  fontSize="small"
                  sx={{ mr: 1.25, color: "text.secondary" }}
                />
                <ListItemText
                  primary={node.name}
                  primaryTypographyProps={{
                    noWrap: true,
                    fontFamily: "var(--cowboy-font-mono)",
                    fontSize: "0.875rem",
                  }}
                />
              </ListItemButton>
              {!isCollapsed && (
                <GitTreeRows
                  nodes={node.children}
                  depth={depth + 1}
                  collapsed={collapsed}
                  onToggle={onToggle}
                  onOpenDiff={onOpenDiff}
                  queue={queue}
                  reviewed={reviewed}
                />
              )}
            </Box>
          );
        }
        const entry = node.entry;
        if (!entry) return null;
        const isReviewed = reviewed.has(
          reviewEntryKey(entry.change.path, entry.scope),
        );
        return (
          <ListItemButton
            key={`${entry.scope}:${entry.change.path}`}
            onClick={() => onOpenDiff(entry, queue)}
            sx={{
              minHeight: 52,
              pl: 1 + depth * 2,
              pr: 1.25,
            }}
          >
            <ListItemIcon
              sx={{
                minWidth: 34,
                color: isReviewed ? "success.main" : "text.secondary",
              }}
            >
              {isReviewed
                ? <CheckCircleOutline fontSize="small" />
                : <ChangeIcon status={entry.change.status} />}
            </ListItemIcon>
            <ListItemText
              primary={node.name}
              primaryTypographyProps={{
                noWrap: true,
                fontFamily: "var(--cowboy-font-mono)",
                fontSize: "0.875rem",
              }}
            />
            <Chip
              size="small"
              label={statusLabel[entry.change.status]}
              color={entry.change.status === "conflicted" ? "error" : "default"}
              sx={{ minWidth: 32 }}
            />
          </ListItemButton>
        );
      })}
    </>
  );
}

export function ReviewChanges({
  sessionId,
  onOpenDiff,
  reviewed,
  onRevision,
  drawer = false,
  onClose,
  refreshToken = 0,
}: {
  sessionId: string | undefined;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  reviewed: ReadonlySet<string>;
  onRevision: (revision: string) => void;
  drawer?: boolean;
  onClose?: () => void;
  refreshToken?: number;
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
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [visibleCount, setVisibleCount] = useState(REVIEW_WINDOW_SIZE);
  const scrollRoot = useRef<HTMLDivElement>(null);
  const loadMoreSentinel = useRef<HTMLDivElement>(null);
  const previousRefreshToken = useRef(refreshToken);
  const sections = useMemo(() => groupGitChanges(changes), [changes]);
  const queue = useMemo(() => reviewQueue(sections), [sections]);
  const sectionCounts = useMemo(
    () => new Map(sections.map((section) => [section.kind, section.entries.length])),
    [sections],
  );
  const visibleSections = useMemo(
    () => limitGitSections(sections, visibleCount),
    [sections, visibleCount],
  );
  const renderedCount = Math.min(visibleCount, queue.length);

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
      setVisibleCount(REVIEW_WINDOW_SIZE);
      setCollapsed(new Set());
      onRevision(result.revision);
    } catch (reason) {
      if (!(reason instanceof DOMException && reason.name === "AbortError")) {
        setError(true);
      }
    } finally {
      if (!signal?.aborted) setLoading(false);
    }
  }, [onRevision, sessionId]);

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load]);

  useEffect(() => {
    if (previousRefreshToken.current === refreshToken) return;
    previousRefreshToken.current = refreshToken;
    void load();
  }, [load, refreshToken]);

  useEffect(() => {
    const root = scrollRoot.current;
    const sentinel = loadMoreSentinel.current;
    if (!root || !sentinel || renderedCount >= queue.length) return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisibleCount((current) =>
            Math.min(current + REVIEW_WINDOW_SIZE, queue.length)
          );
        }
      },
      { root, rootMargin: "480px 0px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [queue.length, renderedCount]);

  return (
    <Stack sx={{ position: "relative", height: "100%", minHeight: 0 }}>
      <Stack
        direction="row"
        alignItems="center"
        sx={{
          px: 2,
          pt: drawer
            ? "calc(env(safe-area-inset-top, 0px) + 18px)"
            : 1,
          pb: 1,
        }}
      >
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
      </Stack>
      {truncated && (
        <Alert severity="info" sx={{ mx: 1.5, mb: 1, py: 0 }}>
          Showing the first 1,000 changes
        </Alert>
      )}
      <Box
        ref={scrollRoot}
        sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.75, pb: 8 }}
      >
        {loading
          ? (
            <Box sx={{ display: "grid", placeItems: "center", pt: 8 }}>
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
                  aria-label="Retry Git changes"
                  onClick={() => void load()}
                >
                  <Refresh fontSize="small" />
                </IconButton>
              }
            >
              Git changes are unavailable
            </Alert>
          )
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
              {visibleSections.map((section) => (
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
                      {sectionCounts.get(section.kind)}
                    </Typography>
                  </Stack>
                  <List
                    disablePadding
                    sx={{
                      borderTop: 1,
                      borderColor: "divider",
                    }}
                  >
                    <GitTreeRows
                      nodes={buildGitChangeTree(section.entries)}
                      depth={0}
                      collapsed={collapsed}
                      onToggle={(path) =>
                        setCollapsed((current) => {
                          const next = new Set(current);
                          if (next.has(path)) next.delete(path);
                          else next.add(path);
                          return next;
                        })}
                      onOpenDiff={onOpenDiff}
                      queue={queue}
                      reviewed={reviewed}
                    />
                  </List>
                </Box>
              ))}
              {renderedCount < queue.length && (
                <Box
                  ref={loadMoreSentinel}
                  role="status"
                  sx={{
                    minHeight: 32,
                    display: "grid",
                    placeItems: "center",
                  }}
                >
                  <Typography variant="caption" color="text.secondary">
                    {`Showing ${renderedCount} of ${queue.length}`}
                  </Typography>
                </Box>
              )}
            </Stack>
          )}
      </Box>
      {drawer && onClose && (
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
          <MobileSheetDismiss onClose={onClose} label="Close Git changes" />
        </Box>
      )}
    </Stack>
  );
}
