import { Button } from "@mui/material";
import { useState } from "react";
import { putSentMessageBack } from "./refusedMessage";

/** "Back to Prompt": the failed turn's sent message, images included, goes
 *  back into the session's empty Composer for editing (refusedMessage.ts). */
export function PutBackButton({
  sessionId,
  beforeKey,
  primary = false,
}: {
  sessionId: string;
  /** The failed turn's row: the message sent before it is put back. */
  beforeKey?: string;
  /** Refused input: putting the message back is the next step. */
  primary?: boolean;
}): React.JSX.Element {
  const [busy, setBusy] = useState(false);
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
        void putSentMessageBack(sessionId, beforeKey).finally(() => setBusy(false));
      }}
      sx={{ minHeight: 28, py: 0.1, px: 1, flexShrink: 0, textTransform: "none" }}
    >
      {busy ? "Restoring…" : "Back to Prompt"}
    </Button>
  );
}
