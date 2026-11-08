import { ArrowBack } from "@mui/icons-material";
import { Box, Button, List, ListItemButton, ListItemText, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { DESKTOP_SURFACE_RADIUS } from "../DesktopEmbeddedControl";
import {
  fetchGitCommit,
  fetchGitRepository,
  type GitCommitDetail,
  type GitCommitSummary,
} from "../../mobile/review/codeApi";
import { REVIEW_TREE_LEFT_EVENT } from "./ReviewFilesList";

const rowSx = {
  py: 0.25,
  px: 1,
  minHeight: desktopSize(30),
  borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px`,
} as const;

/** "3h ago" from an ISO time; the full time is the row's tooltip. */
export function commitAge(iso: string, now = Date.now()): string {
  const minutes = Math.floor(Math.max(0, now - Date.parse(iso)) / 60_000);
  if (!Number.isFinite(minutes)) return "";
  if (minutes < 60) return `${String(minutes)}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${String(hours)}h ago`;
  return `${String(Math.floor(hours / 24))}d ago`;
}

function CommitFiles({
  sessionId,
  commit,
  openPath,
  onBack,
  onOpen,
}: {
  sessionId: string;
  commit: GitCommitSummary;
  openPath: string | undefined;
  onBack: () => void;
  onOpen: (oid: string, path: string) => void;
}): React.JSX.Element {
  const [detail, setDetail] = useState<GitCommitDetail>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    void fetchGitCommit(sessionId, commit.oid, controller.signal).then(setDetail, () => {
      if (!controller.signal.aborted) setFailed(true);
    });
    return () => controller.abort();
  }, [commit.oid, sessionId]);
  return (
    <>
      <ListItemButton data-desktop-item="history-back" data-review-history-back onClick={onBack} sx={rowSx}>
        <ArrowBack sx={{ fontSize: "1rem", mr: 1 }} />
        <ListItemText
          primary={commit.subject}
          secondary={`${commit.oid.slice(0, 10)} · ${commit.author}`}
          slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.8rem", fontWeight: 650 } }, secondary: { noWrap: true, sx: { fontSize: "0.7rem" } } }}
        />
      </ListItemButton>
      {failed && <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>The commit could not be read.</Typography>}
      {detail?.files.map((file) => (
        <ListItemButton
          key={file.path}
          data-desktop-item={`${commit.oid}:${file.path}`}
          data-review-commit-file={file.path}
          selected={file.path === openPath}
          onClick={() => onOpen(commit.oid, file.path)}
          sx={{ ...rowSx, pl: 3 }}
        >
          <ListItemText
            primary={file.path}
            secondary={file.oldPath ? `from ${file.oldPath}` : file.status}
            slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.78rem", direction: "rtl", textAlign: "left" } }, secondary: { sx: { fontSize: "0.68rem" } } }}
          />
        </ListItemButton>
      ))}
      {detail?.filesTruncated && (
        <Typography sx={{ px: 2, py: 1, fontSize: "0.7rem", color: "text.secondary" }}>More files than shown.</Typography>
      )}
    </>
  );
}

/**
 * History: the branch's commits, newest first, paged in as the reader
 * reaches the end. `L`/`Enter` opens a commit's files, `H` returns.
 */
export function ReviewHistoryList({
  sessionId,
  dataRevision,
  openCommitPath,
  onOpen,
}: {
  sessionId: string;
  dataRevision: number;
  openCommitPath: { oid: string; path: string } | undefined;
  onOpen: (oid: string, path: string) => void;
}): React.JSX.Element {
  const [commits, setCommits] = useState<readonly GitCommitSummary[]>([]);
  const [more, setMore] = useState(false);
  const [failed, setFailed] = useState(false);
  const [selected, setSelected] = useState<GitCommitSummary>();
  const list = useRef<HTMLUListElement>(null);
  const load = (after?: string): void => {
    void fetchGitRepository(sessionId, undefined, after).then((page) => {
      setCommits((previous) => after ? [...previous, ...page.commits] : page.commits);
      setMore(page.historyTruncated);
    }, () => setFailed(true));
  };
  useEffect(() => {
    setSelected(undefined);
    setFailed(false);
    load();
    // Reload when the context or the worktree changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, dataRevision]);
  // `H` on a commit's file list goes back to the commits (REVIEW_TREE_LEFT_EVENT).
  const back = useRef<() => void>(() => {});
  // The row to focus once the list it belongs to has rendered.
  const focusAfter = useRef<string | undefined>(undefined);
  back.current = () => {
    focusAfter.current = selected ? `[data-review-commit="${CSS.escape(selected.oid)}"]` : undefined;
    setSelected(undefined);
  };
  useEffect(() => {
    const selector = focusAfter.current;
    focusAfter.current = undefined;
    if (selector) list.current?.querySelector<HTMLElement>(selector)?.focus();
  }, [selected]);
  useEffect(() => {
    const element = list.current;
    const listener = (event: Event): void => {
      if (!selected) return;
      event.preventDefault();
      back.current();
    };
    element?.addEventListener(REVIEW_TREE_LEFT_EVENT, listener);
    return () => element?.removeEventListener(REVIEW_TREE_LEFT_EVENT, listener);
  }, [selected]);
  return (
    <List ref={list} dense disablePadding data-review-history sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.5 }}>
      {selected
        ? (
          <CommitFiles
            sessionId={sessionId}
            commit={selected}
            openPath={openCommitPath?.oid === selected.oid ? openCommitPath.path : undefined}
            onBack={() => back.current()}
            onOpen={onOpen}
          />
        )
        : (
          <>
            {failed && commits.length === 0 && (
              <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>History could not be read.</Typography>
            )}
            {commits.map((commit) => (
              <ListItemButton
                key={commit.oid}
                data-desktop-item={commit.oid}
                data-review-commit={commit.oid}
                onClick={() => {
                  focusAfter.current = "[data-review-history-back]";
                  setSelected(commit);
                }}
                title={`${commit.oid}\n${new Date(commit.authoredAt).toLocaleString()}`}
                sx={rowSx}
              >
                <ListItemText
                  primary={commit.subject}
                  secondary={
                    <Box component="span" sx={{ display: "flex", gap: 0.75 }}>
                      <Box component="span" sx={{ fontFamily: "monospace" }}>{commit.oid.slice(0, 7)}</Box>
                      <Box component="span" sx={{ overflow: "hidden", textOverflow: "ellipsis" }}>{commit.author}</Box>
                      <Box component="span" sx={{ flexShrink: 0 }}>{commitAge(commit.authoredAt)}</Box>
                      {commit.decorations.slice(0, 2).map((decoration) => (
                        <Box component="span" key={decoration} sx={{ color: "primary.main", flexShrink: 0 }}>{decoration}</Box>
                      ))}
                    </Box>
                  }
                  slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.8rem" } }, secondary: { component: "span", sx: { fontSize: "0.68rem" } } }}
                />
              </ListItemButton>
            ))}
            {more && (
              <Button data-desktop-item="history-more" size="small" onClick={() => load(commits.at(-1)?.oid)} sx={{ m: 1 }}>
                Older commits
              </Button>
            )}
          </>
        )}
    </List>
  );
}
