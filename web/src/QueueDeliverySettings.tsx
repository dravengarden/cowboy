import { useState } from "react";
import {
  Alert,
  Box,
  Button,
  FormControlLabel,
  MenuItem,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import { ConfirmSheet } from "./Sheet";
import { setPaused, useStoreSelector } from "./store";
import {
  type DeliveryMode,
  releaseDeliveryHold,
  saveDeliveryPolicy,
  useDeliveryOptions,
} from "./deliveryPolicy";

/** Both Queue and Session Settings edit this single server-owned preference. */
export function QueueDeliverySettings(
  { sessionId, pauseControl = true }: {
    sessionId: string;
    pauseControl?: boolean;
  },
): React.JSX.Element {
  const { options, error } = useDeliveryOptions(sessionId);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [releaseOpen, setReleaseOpen] = useState(false);
  const paused = useStoreSelector((state) =>
    state.sessions.find((session) => session.id === sessionId)?.paused ?? false
  );
  return (
    <Box sx={{ py: 1 }}>
      {pauseControl && (
        <FormControlLabel
          label="Pause queue"
          control={
            <Switch
              checked={paused}
              onChange={(_, checked) => setPaused(sessionId, checked)}
            />
          }
        />
      )}
      <TextField
        select
        fullWidth
        size="small"
        label="Default delivery mode"
        value={options?.policy.mode ?? "next_turn"}
        disabled={!options || saving}
        onChange={(event): void => {
          if (!options || saving) return;
          setSaving(true);
          setFailure(null);
          void saveDeliveryPolicy(sessionId, {
            mode: event.target.value as DeliveryMode,
            revision: options.policy.revision,
          }).catch((reason: unknown) => setFailure(String(reason)))
            .finally(() => setSaving(false));
        }}
      >
        <MenuItem value="next_turn">Next turn</MenuItem>
        <MenuItem value="steer">Steer</MenuItem>
        <MenuItem value="tool_boundary">Next checkpoint</MenuItem>
      </TextField>
      <Typography variant="caption" color="text.secondary">
        Preselects the running-turn picker. Existing queued messages retain
        their mode.
      </Typography>
      {options?.policy.mode !== "next_turn" && !options?.target && (
        <Typography variant="caption" color="text.secondary" display="block">
          Requires an active worker with native delivery support.
        </Typography>
      )}
      {options?.policy.mode === "tool_boundary" && (
        <Typography variant="caption" color="text.secondary" display="block">
          {options.controls.next_checkpoint
            ? "Consume after foreground tools finish, or after generation if no tool boundary remains."
            : "The current worker does not expose a distinct native queue priority. Next turn remains available."}
        </Typography>
      )}
      {(failure ?? error) && (
        <Alert severity="warning">{failure ?? error}</Alert>
      )}
      {options?.hold && (
        <Alert
          severity="warning"
          action={<Button onClick={() => setReleaseOpen(true)}>Inspect</Button>}
        >
          Queue held · interrupt outcome requires reconciliation.
        </Alert>
      )}
      <ConfirmSheet
        open={releaseOpen}
        onClose={() => setReleaseOpen(false)}
        title="Release delivery hold?"
        actions={
          <Button
            disabled={saving || options?.status !== "running"}
            onClick={() => {
              if (!options?.hold) return;
              setSaving(true);
              void releaseDeliveryHold(sessionId, options.hold).then(() =>
                setReleaseOpen(false)
              )
                .catch((reason: unknown) => setFailure(String(reason))).finally(
                  () => setSaving(false),
                );
            }}
          >
            Release hold
          </Button>
        }
      >
        <Typography variant="body2">
          Check the transcript before continuing. Release does not resend the
          replacement message. Existing queued messages may run unless Pause is
          enabled.
        </Typography>
        <Typography variant="caption">Operation: {options?.hold}</Typography>
      </ConfirmSheet>
    </Box>
  );
}
