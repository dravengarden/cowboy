// The product confirmation for a message an app wrote (docs/widget-apps.md
// §7): the exact text, labelled as the app's, shown before anything leaves the
// device. Sending queues exactly that text as the person's own prompt.

import { useState } from "react";
import { Alert, Box, Button, Typography } from "@mui/material";
import { ConfirmSheet } from "../../Sheet";
import { copyText } from "../../clipboard";
import { discardAppMessage, sendAppMessage, type PendingAppMessage } from "./appMessages";

export default function AppMessageConfirm({
  pending,
  title,
  onClose,
}: {
  pending: PendingAppMessage;
  title: string;
  onClose: () => void;
}): React.JSX.Element {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ text: string; outdated: boolean } | null>(null);
  const send = async (): Promise<void> => {
    setBusy(true);
    const reply = await sendAppMessage(pending);
    setBusy(false);
    if (reply.kind === "sent") onClose();
    else setNotice({ text: reply.message, outdated: reply.code === "app_updated" });
  };
  const cancel = (): void => {
    discardAppMessage(pending);
    onClose();
  };
  return (
    <ConfirmSheet
      open
      onClose={cancel}
      title={pending.state === "sending" ? "Send this message again?" : "Send this message?"}
      actions={
        <>
          <Button onClick={cancel} disabled={busy} sx={{ textTransform: "none" }}>
            {pending.state === "sending" ? "Discard" : "Cancel"}
          </Button>
          {notice?.outdated ? (
            <Button
              variant="contained"
              onClick={(): void => {
                void copyText(pending.text);
                cancel();
              }}
              sx={{ textTransform: "none" }}
            >
              Copy for the composer
            </Button>
          ) : (
            <Button
              variant="contained"
              data-app-message-send=""
              disabled={busy}
              onClick={(): void => void send()}
              sx={{ textTransform: "none" }}
            >
              Send
            </Button>
          )}
        </>
      }
    >
      <Box sx={{ display: "grid", gap: 1.5 }} data-app-message-confirm="">
        <Typography variant="caption" color="text.secondary">
          {pending.state === "sending"
            ? `This message from the app “${title}” may not have been sent. Sending again never sends it twice.`
            : `Written by the app “${title}”. The agent receives it as your message:`}
        </Typography>
        <Box
          data-app-message-text=""
          sx={{
            border: 1,
            borderColor: "divider",
            borderRadius: 1,
            px: 1.5,
            py: 1,
            bgcolor: "action.hover",
            whiteSpace: "pre-wrap",
            overflowWrap: "anywhere",
          }}
        >
          <Typography variant="body2">{pending.text}</Typography>
        </Box>
        {notice ? <Alert severity="warning">{notice.text}</Alert> : null}
      </Box>
    </ConfirmSheet>
  );
}
