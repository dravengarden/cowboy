import { useState } from "react";
import { Alert, Box, Button, CircularProgress, Stack, Typography } from "@mui/material";
import { ErrorOutline } from "@mui/icons-material";
import { SessionContinuedFrom } from "./SessionContextCue";
import { MessagePreview } from "./MessagePreview";
import { QueuedAttachmentChips } from "./QueuedAttachmentChips";
import { attachmentTrayForSurface } from "./composer/attachmentPresentation";
import { prettifyCrashDetail } from "./crashDetail";
import { reloadSession } from "./sessionReload";
import { handoffOperation, SessionHandoffCancel } from "./SessionHandoffCancel";
import { useStoreSelector } from "./store";

/** Startup is a conversation with pending delivery, not an empty-page hero.
 *  A `blocked` startup stopped before its first turn: the same page says why
 *  in place and offers Retry, and queued messages stay where they were. */
export function PreparingConversation({ sessionId, location, blocked }: {
  sessionId: string;
  location: string;
  blocked?: { detail: string | null } | undefined;
}): React.JSX.Element {
  const queue = useStoreSelector((state) => state.queues.get(sessionId));
  const continued = useStoreSelector((state) =>
    state.sessions.some((session) => session.id === sessionId && Boolean(session.continued_from))
  );
  return (
    <Stack data-conversation-empty-state={blocked ? "blocked" : "preparing"} spacing={3} sx={{ flex: "1 1 0 !important", minHeight: 180, width: "100%", px: 2, py: 3, minWidth: 0, alignItems: "center", justifyContent: "safe center" }}>
      {queue?.map((row) => {
        // Only the continuation's own first message can still take the switch back.
        const operation = continued && row.status === undefined ? handoffOperation(row.cmid) : undefined;
        return (
        <Box key={row.id} sx={{ alignSelf: "flex-end", width: "100%", maxWidth: 560, minWidth: 0, overflowWrap: "anywhere" }}>
          <MessagePreview text={row.text} attachments={row.attachments} />
          <QueuedAttachmentChips attachments={attachmentTrayForSurface(row.attachments, row.text)} />
          <Stack direction="row" spacing={1} alignItems="center" justifyContent="space-between" sx={{ minWidth: 0 }}>
            <Typography variant="caption" color={row.status === "failed" && !blocked ? "error" : "text.secondary"}>
              {blocked ? "Queued · sends once the session starts" : row.status === "failed" ? row.failure || "Not sent — check the queue" : "Waiting to send"}
            </Typography>
            {operation !== undefined && <SessionHandoffCancel sessionId={sessionId} operation={operation} />}
          </Stack>
        </Box>
        );
      })}
      <Stack spacing={1.5} alignItems="center" sx={{ width: "100%", maxWidth: blocked ? "28rem" : "22rem", textAlign: "center" }}>
        {blocked ? <BlockedStartup sessionId={sessionId} location={location} detail={blocked.detail} /> : (
          <Stack role="status" spacing={0.75} alignItems="center">
            <Stack direction="row" spacing={1} alignItems="center">
              <CircularProgress size="1rem" thickness={3} sx={{ color: "text.secondary" }} />
              <Typography sx={{ fontSize: "1rem", fontWeight: 500 }}>Preparing session</Typography>
            </Stack>
            {location && <Typography variant="caption" color="text.secondary">{location}</Typography>}
          </Stack>
        )}
        <SessionContinuedFrom sessionId={sessionId} />
      </Stack>
    </Stack>
  );
}

function BlockedStartup({ sessionId, location, detail }: {
  sessionId: string;
  location: string;
  detail: string | null;
}): React.JSX.Element {
  const [retrying, setRetrying] = useState(false);
  const [error, setError] = useState("");
  const retry = (): void => {
    setRetrying(true);
    setError("");
    void reloadSession(sessionId)
      .catch((failure: unknown) => setError(failure instanceof Error ? failure.message : "Retry did not start"))
      .finally(() => setRetrying(false));
  };
  return (
    <Stack role="alert" spacing={1} alignItems="center" sx={{ width: "100%" }}>
      <Stack direction="row" spacing={1} alignItems="center">
        <ErrorOutline fontSize="small" color="error" />
        <Typography sx={{ fontSize: "1rem", fontWeight: 500 }}>Session didn't start</Typography>
      </Stack>
      {location && <Typography variant="caption" color="text.secondary">{location}</Typography>}
      <Typography variant="body2" color="text.secondary" sx={{ overflowWrap: "anywhere" }}>
        {detail ? prettifyCrashDetail(detail) : "The agent stopped before its first turn."}
      </Typography>
      <Button data-session-start-retry variant="contained" size="small" disableElevation disabled={retrying} onClick={retry}>
        {retrying ? "Retrying…" : "Retry"}
      </Button>
      {error && <Alert severity="error" sx={{ width: "100%", textAlign: "left" }}>{error}</Alert>}
    </Stack>
  );
}
