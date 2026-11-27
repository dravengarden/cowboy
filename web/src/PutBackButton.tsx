import { Button } from "@mui/material";
import { createContext, useContext, useState } from "react";
import { putSentMessageBack } from "./refusedMessage";
import { useStoreSelector } from "./store";

/** Whether this transcript's session has a reachable Composer. Off by
 *  default: call, subagent and view-only transcripts never offer recovery;
 *  the session's own transcript turns it on (App's StoreTranscript). */
export const ComposerRecovery = createContext(false);

/** "Back to Prompt": the failed turn's sent message, images included, goes
 *  back into the session's empty Composer for editing (refusedMessage.ts). */
export function PutBackButton({
  sessionId,
  failureKey,
  primary = false,
}: {
  sessionId: string;
  /** The failed turn's row: the message sent before it is put back. */
  failureKey: string;
  /** Refused input: putting the message back is the next step. */
  primary?: boolean;
}): React.JSX.Element | null {
  const enabled = useContext(ComposerRecovery);
  // The loaded history's dense start: older events are retained markers only.
  const coveredFrom = useStoreSelector((snapshot) => {
    const pagination = snapshot.pagination.get(sessionId);
    return pagination && !pagination.reachedStart ? pagination.beforeSeq : null;
  });
  const [busy, setBusy] = useState(false);
  if (!enabled) return null;
  return (
    <Button
      size="small"
      variant={primary ? "outlined" : "text"}
      color="inherit"
      disabled={busy}
      data-turn-failure-put-back
      title="Put the message you sent back in the Prompt to edit and send it again"
      onClick={(): void => {
        setBusy(true);
        void putSentMessageBack(sessionId, failureKey, coveredFrom).finally(() => setBusy(false));
      }}
      sx={{ minHeight: 28, py: 0.1, px: 1, flexShrink: 0, textTransform: "none" }}
    >
      {busy ? "Restoring…" : "Back to Prompt"}
    </Button>
  );
}
