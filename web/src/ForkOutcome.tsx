import { Checkbox, FormControlLabel, Stack, Typography } from "@mui/material";

/** What each Replace / Fork action does with the selected target, plus the
 *  worktree uncommitted-changes option for Linked Forks. */
export function ForkOutcome({
  forkOnly,
  allowFork,
  sameAccount,
  samePlacement,
  copiesWorktree,
  supportsWorkspaceMode,
  forkMode,
  waitForTurn,
  sourceBusy,
  copyUncommitted,
  onCopyUncommitted,
}: {
  forkOnly: boolean;
  allowFork: boolean;
  sameAccount: boolean;
  samePlacement: boolean;
  copiesWorktree: boolean;
  supportsWorkspaceMode: boolean;
  forkMode: "fork" | "linked";
  waitForTurn: boolean;
  sourceBusy: boolean;
  copyUncommitted: boolean;
  onCopyUncommitted: (value: boolean) => void;
}): React.JSX.Element {
  return (
    <Stack spacing={0.5} data-new-session-outcome>
      {!forkOnly && (
        <Typography variant="caption" color="text.secondary" data-new-session-continue>
          {sameAccount && samePlacement
            ? `Replace starts a new session with a fresh conversation on your next message${
                copiesWorktree ? ", with uncommitted changes" : ""
              }. This session then moves to Trash.`
            : copiesWorktree
              ? "Replace continues in a new session with this agent on your next message, with uncommitted changes. This session then moves to Trash."
              : "Replace continues in a new session with this agent on your next message. This session then moves to Trash and keeps uncommitted changes."}
        </Typography>
      )}
      {allowFork && (
        <Typography
          variant="caption"
          color="text.secondary"
          data-new-session-fork={!samePlacement ? "unavailable" : forkMode}
        >
          {!samePlacement
            ? "Fork needs this project and Machine."
            : forkMode === "fork"
              ? "Fork starts a new session now and keeps this one, copying the conversation and uncommitted changes."
              : copiesWorktree
                ? !supportsWorkspaceMode
                  ? "Fork starts a new session now with a handoff digest of this conversation and a copy of this worktree."
                  : sourceBusy || !copyUncommitted
                    ? "Fork starts a new session now from this session's committed HEAD and keeps this one. The new agent receives a handoff digest of this conversation."
                    : "Fork starts a new session now with a handoff digest of this conversation and a copy of uncommitted changes."
                : "Fork starts a new session now and keeps this one. The new agent reads this conversation; uncommitted changes stay here."}
          {samePlacement && waitForTurn && " Available after the current turn."}
        </Typography>
      )}
      {allowFork && samePlacement && copiesWorktree && supportsWorkspaceMode && forkMode === "linked" && (
        <FormControlLabel
          control={
            <Checkbox
              size="small"
              checked={!sourceBusy && copyUncommitted}
              disabled={sourceBusy}
              onChange={(_, checked) => onCopyUncommitted(checked)}
              inputProps={{ "data-fork-copy-uncommitted": "true" } as React.InputHTMLAttributes<HTMLInputElement>}
            />
          }
          label={
            <Typography variant="caption" color="text.secondary">
              {sourceBusy
                ? "Copy uncommitted changes (source is running — branching from committed HEAD)"
                : "Copy uncommitted changes into new worktree"}
            </Typography>
          }
          sx={{ ml: -0.5, my: -0.25 }}
        />
      )}
    </Stack>
  );
}
