import { Alert, Button, Stack, Typography } from "@mui/material";
import { useEffect, useState, useSyncExternalStore } from "react";
import { ConfirmSheet } from "./Sheet";
import { stripImageTokens } from "./attachments";
import {
  cancelPriorSendDecisions,
  currentPriorSendDecision,
  finishPriorSendDecision,
  type PriorSendDecision,
  subscribePriorSendDecision,
} from "./priorSendDecision";
import {
  hasPendingPriorSends,
  heldDeliveryDetails,
  retryQueued,
  saveHeldDeliveryAsDraft,
  useHeldDeliveries,
  useSessionObligations,
} from "./store";

export function PriorSendDecisionSheet(): React.JSX.Element | null {
  useEffect(() => () => cancelPriorSendDecisions(), []);
  const request = useSyncExternalStore(
    subscribePriorSendDecision,
    currentPriorSendDecision,
    () => null,
  );
  return request
    ? <Decision key={request.ids.join(":")} request={request} />
    : null;
}

function Decision(
  { request }: { request: PriorSendDecision },
): React.JSX.Element | null {
  useHeldDeliveries();
  useSessionObligations(request.sessionId);
  const rows = heldDeliveryDetails(request.sessionId).filter((row) =>
    request.ids.includes(row.id)
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const pending = hasPendingPriorSends(request.sessionId, request.ids);
  useEffect(() => {
    if (!pending && !saving) finishPriorSendDecision(request, true);
  }, [pending, saving, request]);
  const continueSend = async (): Promise<void> => {
    setSaving(true);
    setError("");
    try {
      for (const row of rows) {
        await saveHeldDeliveryAsDraft(request.sessionId, row.id);
      }
      finishPriorSendDecision(request, true);
    } catch {
      setError(
        "Could not keep the old message. Nothing new was sent; please try again.",
      );
      setSaving(false);
    }
  };
  if (!pending && !saving) return null;
  return (
    <ConfirmSheet
      open
      title={rows.length === 1
        ? "Previous message unconfirmed"
        : "Previous messages unconfirmed"}
      onClose={() => {
        if (!saving) finishPriorSendDecision(request, false);
      }}
      actions={
        <Stack direction="row" spacing={1} sx={{ width: "100%" }}>
          <Button
            fullWidth
            disabled={saving}
            onClick={() => {
              rows.forEach((row) => retryQueued(request.sessionId, row.id));
              finishPriorSendDecision(request, false);
            }}
          >
            Retry old
          </Button>
          <Button
            fullWidth
            variant="contained"
            disabled={saving}
            onClick={() => void continueSend()}
          >
            {saving ? "Saving…" : "Ignore & send"}
          </Button>
        </Stack>
      }
    >
      <Stack spacing={1}>
        <Typography variant="body2">
          Retry the earlier message, or send your new one. Ignored messages are
          kept in Drafts.
        </Typography>
        {rows.slice(0, 2).map((row) => (
          <Stack
            key={row.id}
            sx={{ pl: 1, borderLeft: 2, borderColor: "divider" }}
          >
            <Typography
              variant="body2"
              color="text.secondary"
              sx={{
                display: "-webkit-box",
                WebkitLineClamp: 2,
                WebkitBoxOrient: "vertical",
                overflow: "hidden",
                overflowWrap: "anywhere",
              }}
            >
              {stripImageTokens(row.text).trim() || "Message with attachments"}
            </Typography>
            {row.attachments > 0 && (
              <Typography variant="caption" color="text.secondary">
                {row.attachments}{" "}
                {row.attachments === 1 ? "attachment" : "attachments"}
              </Typography>
            )}
          </Stack>
        ))}
        {rows.length > 2 && (
          <Typography variant="caption">And {rows.length - 2} more</Typography>
        )}
        {error && <Alert severity="error">{error}</Alert>}
      </Stack>
    </ConfirmSheet>
  );
}
