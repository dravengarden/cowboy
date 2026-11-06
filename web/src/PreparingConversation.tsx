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
    <Stack data-conversation-empty-state="preparing" spacing={3} sx={{ flex: "1 1 0 !important", minHeight: 180, width: "100%", px: 2, py: 3, minWidth: 0, alignItems: "center", justifyContent: "safe center" }}>
      {queue?.map((row) => (
        <Box key={row.id} sx={{ alignSelf: "flex-end", width: "100%", maxWidth: 560, minWidth: 0, overflowWrap: "anywhere" }}>
          <MessagePreview text={row.text} attachments={row.attachments} />
          <QueuedAttachmentChips attachments={attachmentTrayForSurface(row.attachments, row.text)} />
          <Typography variant="caption" color={row.status === "failed" ? "error" : "text.secondary"}>
            {row.status === "failed" ? row.failure || "Not sent — check the queue" : "Waiting to send"}
          </Typography>
        </Box>
      ))}
      <Stack spacing={1.5} alignItems="center" sx={{ width: "100%", maxWidth: "22rem", textAlign: "center" }}>
        <Stack role="status" spacing={0.75} alignItems="center">
          <Stack direction="row" spacing={1} alignItems="center">
            <CircularProgress size="1rem" thickness={3} sx={{ color: "text.secondary" }} />
            <Typography sx={{ fontSize: "1rem", fontWeight: 500 }}>Preparing session</Typography>
          </Stack>
          {location && <Typography variant="caption" color="text.secondary">{location}</Typography>}
        </Stack>
        <SessionContinuedFrom sessionId={sessionId} />
      </Stack>
    </Stack>
  );
}
