import { useState } from "react";
import { Alert, Button, CircularProgress, Stack, Typography } from "@mui/material";
import { ErrorOutline } from "@mui/icons-material";
import { SessionContinuedFrom } from "./SessionContextCue";
import { prettifyCrashDetail } from "./crashDetail";
import { reloadSession } from "./sessionReload";
import { handoffOperation, SessionHandoffCancel } from "./SessionHandoffCancel";
import { useStoreSelector } from "./store";

/** Startup status for a session whose first message is already on its way.
 *  The queued messages themselves live only in the Composer's Queue panel,
 *  which can edit, delete or send them; repeating them here showed the same
 *  text twice. A continuation whose first message still waits for its agent
 *  keeps the one action the panel cannot offer: taking the switch back.
 *  A `blocked` startup stopped before its first turn: the same page says why
 *  in place and offers Retry, and queued messages stay where they were. */
export function PreparingConversation({ sessionId, location, blocked }: {
  sessionId: string;
  location: string;
  blocked?: { detail: string | null } | undefined;
}): React.JSX.Element {
  const operation = useStoreSelector((state) => {
    const continued = state.sessions.some((session) => session.id === sessionId && Boolean(session.continued_from));
    if (!continued) return undefined;
    // Only the continuation's own first message can still take the switch back.
    for (const row of state.queues.get(sessionId) ?? []) {
      const found = row.status === undefined ? handoffOperation(row.cmid) : undefined;
      if (found !== undefined) return found;
    }
    return undefined;
  });
  return (
    <Stack data-conversation-empty-state={blocked ? "blocked" : "preparing"} spacing={3} sx={{ flex: "1 1 0 !important", minHeight: 180, width: "100%", px: 2, py: 3, minWidth: 0, alignItems: "center", justifyContent: "safe center" }}>
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
        {operation !== undefined && <SessionHandoffCancel sessionId={sessionId} operation={operation} />}
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
