import { Box, Button, Typography } from "@mui/material";
import { useEffect } from "react";
import { expandComposerStackPanel } from "./composerStackAccordion";
import { useNetworkActionState } from "./NetworkActionFeedback";
import { loadStagedQueuePrompt } from "./stagedQueuePrompt";
import { confirmedPending, reconcileStagedQueue, useStoreSelector } from "./store";

const buttonSx = { flexShrink: 0, minWidth: 0, textTransform: "none" };

export function NextAgentCue(
  { sessionId, revision, label }: { sessionId: string; revision: number; label: string },
): React.JSX.Element {
  // A previous Send now could have put an optimistic bubble over the queued
  // row. The Controller's queue still owns it until the move is confirmed.
  const queue = useStoreSelector(() => confirmedPending(sessionId)?.queue);
  const visibleCount = useStoreSelector((state) => state.queues.get(sessionId)?.length ?? 0);
  const move = useNetworkActionState();
  const count = Math.max(queue?.length ?? 0, visibleCount);
  const message = count === 1 ? queue?.[0] : undefined;
  useEffect(() => {
    if (queue?.length) void reconcileStagedQueue(sessionId, revision).catch(() => undefined);
  }, [queue, sessionId, revision]);
  const recover = (): Promise<void> | void => {
    if (message) return loadStagedQueuePrompt(sessionId, revision, message);
    expandComposerStackPanel("queued");
  };
  const blocker = count > 0 ? `${count} queued · ${label}` : `Starts ${label}`;
  return (
    <Box sx={{ display: "flex", flex: 1, minWidth: 0, alignItems: "center", gap: 0.5 }}>
      <Typography
        variant="caption"
        noWrap
        color="text.secondary"
        title={count ? `Resolve the queue before sending to ${label}` : blocker}
        sx={{ flex: 1, minWidth: 0 }}
      >
        {blocker}
      </Typography>
      {(count > 0 || move.pending) && (
        <Button
          size="small"
          color="inherit"
          disabled={move.pending}
          onClick={() => void move.run(recover)}
          aria-label={message ? "Use queued message in Composer" : "Review queued messages"}
          sx={buttonSx}
        >
          {move.pending ? "Loading…" : message ? "Use message" : "Review queue"}
        </Button>
      )}
    </Box>
  );
}
