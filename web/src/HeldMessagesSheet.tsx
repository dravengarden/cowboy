import { Alert, Button, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { ConfirmSheet } from "./Sheet";
import {
  heldDeliveryDetails,
  retryQueued,
  saveHeldDeliveryAsDraft,
  useHeldDeliveries,
} from "./store";

/** Review the local retry obligations without navigating away from a session. */
export function HeldMessagesSheet({ sessionId, onClose }: {
  sessionId: string;
  onClose: () => void;
}): React.JSX.Element {
  useHeldDeliveries();
  const rows = heldDeliveryDetails(sessionId);
  const [saving, setSaving] = useState<string | null>(null);
  const [error, setError] = useState("");
  const saveDraft = async (id: string): Promise<void> => {
    setSaving(id);
    setError("");
    try {
      await saveHeldDeliveryAsDraft(sessionId, id);
    } catch {
      setError(
        "Could not save the draft. The original message is still kept; try again.",
      );
    } finally {
      setSaving(null);
    }
  };
  return (
    <ConfirmSheet
      open
      onClose={onClose}
      title="Unconfirmed messages"
      actions={<Button onClick={onClose}>Close</Button>}
    >
      <Stack spacing={1.5}>
        <Typography variant="body2" color="text.secondary">
          These messages are kept on this device because delivery was not
          confirmed. A connected session does not confirm an earlier message.
          Retry sending, or save a message to drafts to keep it for
          later.
        </Typography>
        {error && <Alert severity="error">{error}</Alert>}
        {rows.length === 0 && (
          <Alert severity="success">No messages need attention.</Alert>
        )}
        {rows.map((row) => (
          <Stack
            key={row.id}
            spacing={1}
            sx={{ p: 1.5, border: 1, borderColor: "divider", borderRadius: 2 }}
          >
            <Typography
              variant="body2"
              sx={{
                whiteSpace: "pre-wrap",
                overflowWrap: "anywhere",
                maxHeight: 160,
                overflowY: "auto",
              }}
            >
              {row.text || (row.attachments
                ? "Message with attachments"
                : "A change is waiting for confirmation")}
            </Typography>
            {row.attachments > 0 && (
              <Typography variant="caption" color="text.secondary">
                {row.attachments}{" "}
                {row.attachments === 1 ? "attachment" : "attachments"}{" "}
                kept with this message
              </Typography>
            )}
            <Stack direction="row" spacing={1}>
              <Button
                disabled={saving !== null}
                onClick={() => retryQueued(sessionId, row.id)}
              >
                Retry
              </Button>
              {row.canSaveDraft && (
                <Button
                  disabled={saving !== null}
                  onClick={() => void saveDraft(row.id)}
                >
                  {saving === row.id ? "Saving…" : "Save to drafts"}
                </Button>
              )}
            </Stack>
          </Stack>
        ))}
      </Stack>
    </ConfirmSheet>
  );
}
