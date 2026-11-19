import { Box, Button, Typography } from "@mui/material";
import { useEffect } from "react";
import { expandComposerStackPanel } from "./composerStackAccordion";
import { checkComposerAvailable, composePendingPrompt } from "./contextPendingPrompt";
import { useNetworkActionState } from "./NetworkActionFeedback";
import { loadSessionContext, stagedNextAgent } from "./sessionContext";
import { confirmedPending, notify, reconcileStagedQueue, sendWithAck, useStoreSelector } from "./store";

const buttonSx = { flexShrink: 0, minWidth: 0, textTransform: "none" };

export function NextAgentCue(
  { sessionId, revision, label }: { sessionId: string; revision: number; label: string },
): React.JSX.Element {
  // A previous Send now could have put an optimistic bubble over the queued
  // row. The Controller's queue still owns it until the move is confirmed.
  const queue = useStoreSelector(() => confirmedPending(sessionId)?.queue);
  const move = useNetworkActionState();
  const count = queue?.length ?? 0;
  useEffect(() => {
    if (queue?.length) void reconcileStagedQueue(sessionId, revision).catch(() => undefined);
  }, [queue, sessionId, revision]);
  const recover = async (): Promise<void> => {
    if (count !== 1 || !queue?.[0]) {
      expandComposerStackPanel("queued");
      return;
    }
    const { id, cmid } = queue[0];
    const savedDraft = () =>
      confirmedPending(sessionId)?.drafts.find((row) => row.id === id && row.cmid === cmid);
    const requireStagedAgent = async () => {
      const context = await loadSessionContext(sessionId);
      if (!stagedNextAgent(context) || context.revision !== revision) {
        throw new Error("The selected agent changed. Review the current switch before sending.");
      }
    };
    checkComposerAvailable(sessionId);
    await requireStagedAgent();
    await sendWithAck(
      { type: "queued_to_draft", session_id: sessionId, id },
      () => savedDraft() !== undefined,
      "Return message to Drafts",
    );
    await requireStagedAgent();
    const draft = savedDraft();
    if (!draft) throw new Error("This draft changed elsewhere. Review it in Drafts.");
    composePendingPrompt(sessionId, draft);
    notify("Message loaded in the Composer. A copy is saved in Drafts.", "info");
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
          aria-label={count <= 1 ? "Use queued message in Composer" : "Review queued messages"}
          sx={buttonSx}
        >
          {move.pending ? "Loading…" : count === 1 ? "Use message" : "Review queue"}
        </Button>
      )}
    </Box>
  );
}
