import { useCallback, useMemo, useState } from "react";
import {
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
} from "@mui/material";
import { resolveSessionAction, type SessionAction } from "../agentCommands";
import { compactTooltip, StopConfirmDialog } from "../Composer";
import { ConfirmConsequence } from "../ConfirmConsequence";
import { importantHaptic } from "../haptic";
import { Kbd, useConfirmEnter } from "../Kbd";
import { NetworkButton } from "../NetworkActionFeedback";
import { ENTER_LABEL, MOD_LABEL } from "../platform";
import type { Status } from "../protocol";
import {
  cancelTurn,
  resetSession,
  submitPrompt,
  useStoreSelector,
} from "../store";
import { useCompactionContext } from "../useCompactionContext";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  desktopLeaderSequence,
} from "./commands/workspaceShortcuts";
import {
  desktopTopBarTimelineSlice,
  sameDesktopTopBarTimelineSlice,
} from "./desktopTopBarTimelineSlice";

export interface DesktopSessionActions {
  /** Absent when the Provider has no compact command. */
  compact?: {
    used: number;
    size: number;
    running: boolean;
    title: string;
    open: () => void;
  };
  /** Absent when the Provider has no clear command. */
  clear?: { open: () => void };
  stop: { busy: boolean; open: () => void };
  /** The confirmations; render once beside the controls. */
  dialogs: React.JSX.Element;
}

/**
 * Compact, Clear and Stop act on the conversation the Prompt writes into, so
 * they live in its delivery row. Keys: `␣PC` and `␣PX` from any focus (the
 * Prompt group, beside `␣PP`), `Mod+.` from anywhere including editors.
 */
export function useDesktopSessionActions(
  sessionId: string,
  status: Status,
): DesktopSessionActions {
  const session = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId)
  );
  const timeline = useStoreSelector(
    (snapshot) => desktopTopBarTimelineSlice(snapshot.timelines.get(sessionId)),
    sameDesktopTopBarTimelineSlice,
  );
  const [open, setOpen] = useState<"compact" | "clear" | "stop" | null>(null);
  const close = useCallback((): void => setOpen(null), []);
  const resolve = (kind: "compact" | "clear"): SessionAction | null =>
    resolveSessionAction(
      kind,
      session?.provider ?? "",
      timeline.availableCommands,
      session?.provider_version,
      session?.provider_generation_digest,
    );
  const compactAction = useMemo(() => resolve("compact"), [
    timeline.availableCommands,
    session?.provider,
    session?.provider_version,
    session?.provider_generation_digest,
  ]);
  const clearAction = useMemo(() => resolve("clear"), [
    timeline.availableCommands,
    session?.provider,
    session?.provider_version,
    session?.provider_generation_digest,
  ]);
  const busy = status === "busy";
  const compacting = busy && timeline.compactingTail;
  const context = useCompactionContext({
    sessionId,
    status,
    serverUsed: session?.context_used ?? 0,
    serverSize: session?.context_size ?? 0,
    completionSeq: timeline.completionSeq,
  });
  const queuedAfterTurn = status === "busy" || status === "starting";

  const confirmCompact = useCallback((): void => {
    setOpen(null);
    if (compactAction?.command) {
      context.beginRefresh();
      submitPrompt(sessionId, compactAction.command, []);
    }
  }, [compactAction?.command, context.beginRefresh, sessionId]);
  useConfirmEnter(open === "compact", confirmCompact);
  const confirmClear = useCallback(async (): Promise<void> => {
    setOpen(null);
    await resetSession(sessionId);
  }, [sessionId]);
  useConfirmEnter(open === "clear", () => {
    void confirmClear();
  });

  const hasCompact = compactAction !== null;
  const hasClear = clearAction !== null;
  const compactCommand = useMemo<DesktopCommand>(() => ({
    id: "prompt.compact",
    title: "Compact Conversation",
    group: "Prompt",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.promptCompact),
    leaderAnywhere: true,
    when: () => hasCompact && !compacting,
    disabledReason: compacting
      ? "Compacting…"
      : "This agent has no compact command",
    run: () => setOpen("compact"),
  }), [compacting, hasCompact]);
  useDesktopCommand(compactCommand);
  const clearCommand = useMemo<DesktopCommand>(() => ({
    id: "prompt.clear",
    title: "Clear Conversation",
    group: "Prompt",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.promptClear),
    leaderAnywhere: true,
    when: () => hasClear,
    disabledReason: "This agent has no clear command",
    run: () => setOpen("clear"),
  }), [hasClear]);
  useDesktopCommand(clearCommand);
  const stopCommand = useMemo<DesktopCommand>(() => ({
    id: "session.stop",
    title: "Stop Current Turn",
    group: "Global",
    shortcut: DESKTOP_SHORTCUTS.stop,
    allowInEditor: true,
    when: () => busy,
    disabledReason: "No turn is running",
    // The browser's own Mod+. must never fire while a turn is idle.
    consumeWhenDisabled: true,
    run: () => setOpen("stop"),
  }), [busy]);
  useDesktopCommand(stopCommand);

  const provider = session?.provider ?? "the agent";
  const dialogs = (
    <>
      <Dialog open={open === "compact"} onClose={close} maxWidth="xs" fullWidth>
        <DialogTitle>Compact conversation?</DialogTitle>
        <DialogContent>
          <DialogContentText>{compactAction?.detail}</DialogContentText>
          {compactAction?.command && (
            <ConfirmConsequence>
              Sends{" "}
              <Box
                component="code"
                sx={{
                  px: 0.5,
                  py: 0.125,
                  borderRadius: 0.75,
                  bgcolor: "action.hover",
                }}
              >
                {compactAction.command}
              </Box>{" "}
              to {provider}
              {queuedAfterTurn ? " (queued after the current turn)" : ""}.
            </ConfirmConsequence>
          )}
        </DialogContent>
        <DialogActions>
          <Button
            color="inherit"
            onClick={close}
            sx={{ textTransform: "none" }}
          >
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <Button
            variant="contained"
            onClick={confirmCompact}
            sx={{ textTransform: "none" }}
          >
            Compact
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={open === "clear"} onClose={close} maxWidth="xs" fullWidth>
        <DialogTitle>Clear conversation?</DialogTitle>
        <DialogContent>
          <DialogContentText>{clearAction?.detail}</DialogContentText>
          <ConfirmConsequence tone="error" irreversible>
            Resets {provider} to a fresh context now
            {queuedAfterTurn ? " (ends the current turn)" : ""}.
          </ConfirmConsequence>
        </DialogContent>
        <DialogActions>
          <Button
            color="inherit"
            onClick={close}
            sx={{ textTransform: "none" }}
          >
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <NetworkButton
            variant="contained"
            color="error"
            networkAction={confirmClear}
            sx={{ textTransform: "none" }}
          >
            Clear
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </NetworkButton>
        </DialogActions>
      </Dialog>
      <StopConfirmDialog
        open={open === "stop" && busy}
        onClose={close}
        onConfirm={(): void => {
          importantHaptic();
          cancelTurn(sessionId);
          setOpen(null);
        }}
      />
    </>
  );

  return {
    ...(compactAction && {
      compact: {
        used: context.used,
        size: context.size,
        running: compacting,
        title: context.refreshing
          ? "Updating context usage…"
          : compacting
          ? "Compacting…"
          : compactTooltip(context.used, context.size),
        open: () => setOpen("compact"),
      },
    }),
    ...(clearAction && { clear: { open: () => setOpen("clear") } }),
    stop: { busy, open: () => setOpen("stop") },
    dialogs,
  };
}
