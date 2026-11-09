// The product-owned confirmation for a widget decision (docs/widgets.md §8.1):
// it shows the exact message the Controller composed, which is exactly what
// the agent will receive as the person's next message. An agent `confirm`
// note is shown labelled as the agent's, never as product text.

import { useState } from "react";
import { Alert, Box, Button, Typography } from "@mui/material";
import { ConfirmSheet } from "../Sheet";
import { requestInteraction, type InteractionRequest } from "./widgetInteractions";

export type ConfirmTarget = {
  sessionId: string;
  widgetId: string;
  request: InteractionRequest;
  message: string;
  messageSha256: string;
  agentNote?: string | undefined;
  destructive: boolean;
};

export default function WidgetConfirm({
  target,
  onClose,
}: {
  target: ConfirmTarget;
  onClose: (sent: boolean) => void;
}): React.JSX.Element {
  const [current, setCurrent] = useState(target);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const send = async (): Promise<void> => {
    setBusy(true);
    const reply = await requestInteraction(current.sessionId, current.widgetId, {
      ...current.request,
      message_sha256: current.messageSha256,
    });
    setBusy(false);
    if (reply.kind === "sent") {
      onClose(true);
    } else if (reply.kind === "changed") {
      setCurrent({ ...current, message: reply.message, messageSha256: reply.messageSha256 });
      setNotice("The widget changed. This is the message that would be sent now.");
    } else if (reply.kind === "refused") {
      setNotice(reply.message);
    }
  };
  return (
    <ConfirmSheet
      open
      onClose={(): void => onClose(false)}
      title="Send this reply?"
      actions={
        <>
          <Button onClick={(): void => onClose(false)} disabled={busy} sx={{ textTransform: "none" }}>
            Cancel
          </Button>
          <Button
            variant="contained"
            color={current.destructive ? "error" : "primary"}
            data-widget-confirm-send=""
            disabled={busy}
            onClick={(): void => void send()}
            sx={{ textTransform: "none" }}
          >
            Send
          </Button>
        </>
      }
    >
      <Box sx={{ display: "grid", gap: 1.5 }} data-widget-confirm="">
        {current.agentNote ? (
          <Typography variant="body2" color="text.secondary">
            <Box component="span" sx={{ fontWeight: 600 }}>
              Agent note:{" "}
            </Box>
            {current.agentNote}
          </Typography>
        ) : null}
        <Typography variant="caption" color="text.secondary">
          The agent receives this as your message:
        </Typography>
        <Box
          data-widget-confirm-message=""
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
          <Typography variant="body2">{current.message}</Typography>
        </Box>
        {notice ? <Alert severity="warning">{notice}</Alert> : null}
      </Box>
    </ConfirmSheet>
  );
}
