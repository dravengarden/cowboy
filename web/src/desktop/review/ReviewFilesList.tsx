import { ChevronRight, ExpandMore, InsertDriveFileOutlined } from "@mui/icons-material";
import { Box, List, ListItemButton, ListItemText, Typography } from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { DESKTOP_SURFACE_RADIUS } from "../DesktopEmbeddedControl";
import {
  type CodeTreeEntry,
  fetchCodeSearch,
  fetchCodeTree,
} from "../../review/codeApi";

interface Row {
  readonly entry: CodeTreeEntry;
  readonly depth: number;
}

/** Visible rows of the lazily loaded tree, depth first. */
export function visibleTreeRows(
  children: ReadonlyMap<string, readonly CodeTreeEntry[]>,
  expanded: ReadonlySet<string>,
  path = "",
  depth = 0,
): Row[] {
  const rows: Row[] = [];
  for (const entry of children.get(path) ?? []) {
    rows.push({ entry, depth });
    if (entry.kind === "directory" && expanded.has(entry.path)) {
      rows.push(...visibleTreeRows(children, expanded, entry.path, depth + 1));
    }
  }
  return rows;
}

function ReviewFileMatches({
  files,
  openPath,
  onOpen,
}: {
  files: readonly string[] | undefined;
  openPath: string | undefined;
  onOpen: (path: string) => void;
}): React.JSX.Element {
  return (
    <List dense disablePadding sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.5 }}>
      {files?.length === 0 && (
        <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>No file matches.</Typography>
      )}
      {(files ?? []).map((path) => (
        <ListItemButton
          key={path}
          data-desktop-item={path}
          data-review-tree-path={path}
          selected={path === openPath}
          onClick={() => onOpen(path)}
          sx={{ py: 0.25, minHeight: desktopSize(28), borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px` }}
        >
          <ListItemText
            primary={path}
            slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.8rem", direction: "rtl", textAlign: "left" } } }}
          />
        </ListItemButton>
      ))}
    </List>
  );
}

export const REVIEW_TREE_LEFT_EVENT = "cowboy:review-tree-left";

/**
 * Files: the worktree as a lazily expanded tree, or the files matching the
 * filter. `L`/`Enter` open a file or expand a folder; `H` (the Navigator's
 * command) collapses a folder or moves to its parent.
 */
export function ReviewFilesList({
  sessionId,
  filter,
  openPath,
  dataRevision,
  onOpen,
}: {
  sessionId: string;
  filter: string;
  openPath: string | undefined;
  dataRevision: number;
  onOpen: (path: string) => void;
}): React.JSX.Element {
  const [children, setChildren] = useState<ReadonlyMap<string, readonly CodeTreeEntry[]>>(new Map());
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [matches, setMatches] = useState<{ query: string; files: readonly string[] }>();
  const [error, setError] = useState(false);
  const load = useCallback((path: string, refresh = false): void => {
    void fetchCodeTree(sessionId, path, undefined, refresh).then((page) => {
      setChildren((previous) => new Map(previous).set(path, page.entries));
    }, () => setError(true));
  }, [sessionId]);
  useEffect(() => {
    setChildren(new Map());
    setExpanded(new Set());
    setError(false);
    load("");
  }, [load]);
  // A worktree change revalidates what is open, keeping the folds.
  useEffect(() => {
    if (dataRevision === 0) return;
    load("", true);
    for (const path of expanded) load(path, true);
    // `expanded` is read at the moment the worktree changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dataRevision, load]);
  const query = filter.trim();
  useEffect(() => {
    if (!query) return undefined;
    const controller = new AbortController();
    const timer = globalThis.setTimeout(() => {
      void fetchCodeSearch(sessionId, query, controller.signal).then(
        (result) => setMatches({ query, files: result.files }),
        () => undefined,
      );
    }, 150);
    return () => {
      globalThis.clearTimeout(timer);
      controller.abort();
    };
  }, [query, sessionId]);
  const toggle = (path: string): void => {
    setExpanded((previous) => {
      const next = new Set(previous);
      if (next.has(path)) next.delete(path);
      else {
        next.add(path);
        if (!children.has(path)) load(path);
      }
      return next;
    });
  };
  const rows = useMemo(() => visibleTreeRows(children, expanded), [children, expanded]);
  const list = useRef<HTMLUListElement>(null);
  const left = useRef<(event: Event) => void>(() => {});
  left.current = (event: Event): void => {
    const row = (event.target as HTMLElement | null)?.closest<HTMLElement>("[data-review-tree-path]");
    const path = row?.dataset.reviewTreePath;
    if (!path) return;
    if (row.dataset.reviewTreeDirectory === "true" && expanded.has(path)) {
      toggle(path);
      return;
    }
    const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
    if (!parent) return;
    event.preventDefault();
    document.querySelector<HTMLElement>(`[data-review-tree-path="${CSS.escape(parent)}"]`)?.focus();
  };
  useEffect(() => {
    const element = list.current;
    const listener = (event: Event): void => left.current(event);
    element?.addEventListener(REVIEW_TREE_LEFT_EVENT, listener);
    return () => element?.removeEventListener(REVIEW_TREE_LEFT_EVENT, listener);
  }, [query]);
  if (query) {
    return (
      <ReviewFileMatches
        files={matches?.query === query ? matches.files : undefined}
        openPath={openPath}
        onOpen={onOpen}
      />
    );
  }
  return (
    <List
      dense
      disablePadding
      ref={list}
      sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.5 }}
    >
      {error && rows.length === 0 && (
        <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>The worktree could not be read.</Typography>
      )}
      {rows.map(({ entry, depth }) => {
        const directory = entry.kind === "directory";
        const open = expanded.has(entry.path);
        return (
          <ListItemButton
            key={entry.path}
            data-desktop-item={entry.path}
            data-review-tree-path={entry.path}
            data-review-tree-directory={directory ? "true" : undefined}
            aria-expanded={directory ? open : undefined}
            selected={!directory && entry.path === openPath}
            onClick={() => directory ? toggle(entry.path) : onOpen(entry.path)}
            sx={{
              py: 0,
              pl: 0.5 + depth * 1.5,
              minHeight: desktopSize(26),
              gap: 0.5,
              borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px`,
              opacity: entry.ignored ? 0.55 : 1,
            }}
          >
            {directory
              ? (open ? <ExpandMore sx={{ fontSize: "1rem" }} /> : <ChevronRight sx={{ fontSize: "1rem" }} />)
              : <InsertDriveFileOutlined sx={{ fontSize: "0.9rem", color: "text.secondary" }} />}
            <ListItemText
              primary={entry.name}
              slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.8rem", fontWeight: directory ? 600 : 400 } } }}
              sx={{ my: 0 }}
            />
          </ListItemButton>
        );
      })}
      {rows.length === 0 && !error && <Box sx={{ p: 2 }} />}
    </List>
  );
}
