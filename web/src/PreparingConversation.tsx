import { Box, CircularProgress, Stack, Typography } from "@mui/material";
import { SessionContinuedFrom } from "./SessionContextCue";
import { MessagePreview } from "./MessagePreview";
import { QueuedAttachmentChips } from "./QueuedAttachmentChips";
import { attachmentTrayForSurface } from "./composer/attachmentPresentation";
import { useStoreSelector } from "./store";

/** Startup is a conversation with pending delivery, not an empty-page hero. */
export function PreparingConversation({ sessionId, location }: { sessionId: string; location: string }): React.JSX.Element {
  const queue = useStoreSelector((state) => state.queues.get(sessionId));
  return (
    <Stack data-conversation-empty-state="preparing" spacing={1.5} sx={{ width: "100%", px: 1.5, py: 1.5, minWidth: 0 }}>
      <SessionContinuedFrom sessionId={sessionId} />
      {queue?.map((row) => (
        <Box key={row.id} sx={{ alignSelf: "flex-end", width: "100%", maxWidth: 560, minWidth: 0, overflowWrap: "anywhere" }}>
          <MessagePreview text={row.text} attachments={row.attachments} />
          <QueuedAttachmentChips attachments={attachmentTrayForSurface(row.attachments, row.text)} />
          <Typography variant="caption" color={row.status === "failed" ? "error" : "text.secondary"}>
            {row.status === "failed" ? row.failure || "Not sent — check the queue" : "Waiting to send"}
          </Typography>
        </Box>
      ))}
      <Stack role="status" direction="row" spacing={1} alignItems="center" sx={{ color: "text.secondary" }}>
        <CircularProgress size="0.875rem" color="inherit" />
        <Typography variant="body2">Preparing session{location ? ` · ${location}` : ""}</Typography>
      </Stack>
    </Stack>
  );
}
