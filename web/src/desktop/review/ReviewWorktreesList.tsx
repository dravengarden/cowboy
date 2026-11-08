import { List, ListItemButton, ListItemText, ListSubheader, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { useStoreSelector } from "../../store";
import { worktreeContextId } from "../../reviewDirectoryDefault";
import { DESKTOP_SURFACE_RADIUS } from "../DesktopEmbeddedControl";
import { fetchGitRepository, type GitWorktreeSummary } from "../../mobile/review/codeApi";
import {
  groupRepositoryWorktrees,
  type RepositoryWorktreeRow,
  worktreeTitle,
} from "../../mobile/review/repositoryTargetModel";
import type { DesktopReviewTarget } from "./useDesktopReviewContext";

/** The context value that reviews `row`, or why it cannot be reviewed. */
export function worktreeReviewValue(
  row: RepositoryWorktreeRow,
  target: DesktopReviewTarget,
  localMachine: boolean,
): { value: string } | { reason: string } {
  const owner = row.sessions[0];
  if (owner) return { value: `session::${owner.id}` };
  if (!localMachine) return { reason: "Worktrees on another Machine are read through their session" };
  if (!target.machineId || !target.workspaceId) return { reason: "This repository is not a registered project" };
  return { value: worktreeContextId(target.machineId, target.workspaceId, row.worktree.path) };
}

/**
 * Worktrees: the repository's `git worktree list`, filed by use as on the
 * phone. `L`/`Enter` reviews one; reading it never moves the Agent.
 */
export function ReviewWorktreesList({
  target,
  onReview,
}: {
  target: DesktopReviewTarget;
  onReview: (value: string) => void;
}): React.JSX.Element {
  const [worktrees, setWorktrees] = useState<readonly GitWorktreeSummary[]>();
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const local = useStoreSelector((snapshot) =>
    snapshot.machines.some((machine) => machine.local && machine.id === target.machineId)
  );
  useEffect(() => {
    const controller = new AbortController();
    setWorktrees(undefined);
    void fetchGitRepository(target.sessionId, controller.signal).then(
      (page) => setWorktrees(page.worktrees),
      () => {
        if (!controller.signal.aborted) setWorktrees([]);
      },
    );
    return () => controller.abort();
  }, [target.sessionId]);
  const groups = useMemo(
    () => groupRepositoryWorktrees(worktrees ?? [], sessions, target.machineId),
    [sessions, target.machineId, worktrees],
  );
  const row = (item: RepositoryWorktreeRow, current: boolean): React.JSX.Element => {
    const choice = worktreeReviewValue(item, target, local);
    const owner = item.sessions[0];
    return (
      <ListItemButton
        key={item.worktree.path}
        data-desktop-item={item.worktree.path}
        data-review-worktree={item.worktree.path}
        selected={current}
        disabled={!current && "reason" in choice}
        title={"reason" in choice && !current ? choice.reason : item.worktree.path}
        onClick={() => {
          if (!current && "value" in choice) onReview(choice.value);
        }}
        sx={{ py: 0.25, px: 1, minHeight: desktopSize(30), borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px` }}
      >
        <ListItemText
          primary={worktreeTitle(item.worktree)}
          secondary={owner ? `${item.location} · ${owner.title}` : item.location}
          slotProps={{
            primary: { noWrap: true, sx: { fontSize: "0.8rem", fontWeight: current ? 650 : 500 } },
            secondary: { noWrap: true, sx: { fontSize: "0.68rem" } },
          }}
        />
      </ListItemButton>
    );
  };
  return (
    <List dense disablePadding data-review-worktrees sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.5 }}>
      {worktrees?.length === 0 && (
        <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>No worktrees could be read.</Typography>
      )}
      {groups.current && row(groups.current, true)}
      {groups.sessions.length > 0 && <ListSubheader sx={{ lineHeight: "26px", fontSize: "0.68rem" }}>Other sessions</ListSubheader>}
      {groups.sessions.map((item) => row(item, false))}
      {groups.other.length > 0 && <ListSubheader sx={{ lineHeight: "26px", fontSize: "0.68rem" }}>No session</ListSubheader>}
      {groups.other.map((item) => row(item, false))}
    </List>
  );
}
