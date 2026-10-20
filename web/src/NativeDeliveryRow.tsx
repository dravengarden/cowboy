import { useState } from "react";
import { Box, Button, Typography } from "@mui/material";
import { ConfirmSheet } from "./Sheet";
import { NetworkButton } from "./NetworkActionFeedback";
import { MessagePreview } from "./MessagePreview";
import { QueuedAttachmentChips } from "./QueuedAttachmentChips";
import { attachmentTrayForSurface } from "./composer/attachmentPresentation";
import { discardQueued, type QueuedMessage, retryQueued } from "./store";

/** Pending native input is receipt-owned, never an editable/replayable queue row. */
export function NativeDeliveryRow(
  { sessionId, row }: { sessionId: string; row: QueuedMessage },
): React.JSX.Element {
  const [dismiss, setDismiss] = useState(false);
  const tray = attachmentTrayForSurface(row.attachments, row.text);
  return (
    <Box sx={{ py: 0.75 }}>
      {row.text.trim() !== "" && (
        <MessagePreview text={row.text} attachments={row.attachments} />
      )}
      {tray.length > 0 && <QueuedAttachmentChips attachments={tray} />}
      <Typography variant="caption" color="text.secondary">
        {row.nativeDelivery === "interrupt"
          ? "Interrupt & send"
          : row.nativeDelivery === "next_checkpoint"
          ? "Next checkpoint"
          : "Steer"} · {row.status ?? "pending"}
      </Typography>
      {row.status === "failed" && (
        <>
          <Button
            size="small"
            onClick={() => {
              if (row.cmid) retryQueued(sessionId, row.cmid);
            }}
          >
            Check receipt
          </Button>
          <Button size="small" color="inherit" onClick={() => setDismiss(true)}>
            Dismiss…
          </Button>
        </>
      )}
      {row.failure && (
        <Typography variant="caption" color="warning.main" display="block">
          {row.failure}
        </Typography>
      )}
      <ConfirmSheet
        open={dismiss}
        onClose={() => setDismiss(false)}
        title="Dismiss local delivery receipt?"
        actions={
          <>
            <Button color="inherit" onClick={() => setDismiss(false)}>
              Keep
            </Button>
            <NetworkButton
              color="warning"
              networkAction={async () => {
                if (row.cmid) await discardQueued(sessionId, row.cmid);
                setDismiss(false);
              }}
            >
              Dismiss
            </NetworkButton>
          </>
        }
      >
        <Typography variant="body2">
          This removes the retained local message and receipt entry. It does not
          cancel an admitted request or release a queue hold. Unknown delivery
          may still complete; inspect the transcript before sending the content
          again.
        </Typography>
      </ConfirmSheet>
    </Box>
  );
}
