// The product confirmation for a message an app wrote (docs/widget-apps.md
// §7): the exact text, labelled as the app's, never as Cowboy's. Sending
// queues exactly that text as the person's own prompt.

import { useState } from "react";
import { Alert, Box, Button, Typography } from "@mui/material";
import { ConfirmSheet } from "../../Sheet";
import { copyText } from "../../clipboard";
import { sendAppMessage, type AppMessageReply, type AppMessageTarget } from "./appMessages";

export default function AppMessageConfirm({
  target,
  title,
  preview,
  onClose,
}: {
  target: AppMessageTarget;
  title: string;
  preview: Extract<AppMessageReply, { kind: "preview" }>;
  onClose: () => void;
}): React.JSX.Element {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ text: string; outdated: boolean } | null>(null);
  const send = async (): Promise<void> => {
    setBusy(true);
    const reply = await sendAppMessage(target, preview);
    setBusy(false);
    if (reply.kind === "sent") onClose();
    else if (reply.kind === "refused")
      setNotice({ text: reply.message, outdated: reply.code === "app_updated" });
  };
  return (
    <ConfirmSheet
      open
      onClose={onClose}
      title="Send this message?"
      actions={
        <>
          <Button onClick={onClose} disabled={busy} sx={{ textTransform: "none" }}>
            Cancel
          </Button>
          {notice?.outdated ? (
            <Button
              variant="contained"
              onClick={(): void => {
                void copyText(preview.message);
                onClose();
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
          {`Written by the app “${title}”. The agent receives it as your message:`}
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
          <Typography variant="body2">{preview.message}</Typography>
        </Box>
        {notice ? <Alert severity="warning">{notice.text}</Alert> : null}
      </Box>
    </ConfirmSheet>
  );
}
