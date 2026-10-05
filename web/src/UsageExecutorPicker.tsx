import { desktopSize } from "./surface/desktopSize";
import { useState } from "react";
import {
  CircularProgress,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useProductAuth } from "./auth/ProductAuthGate";
import type { UsageExecutionSettings } from "./usageApi";

export function UsageExecutorPicker({ account, settings, onChange }: {
  account: string;
  settings: UsageExecutionSettings;
  onChange: (account: string, machine: string | null) => Promise<void>;
}): React.JSX.Element | null {
  const { me } = useProductAuth();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const execution = settings.providers[account];
  if (!execution) return null;
  const name = (id: string): string =>
    settings.machines.find((machine) => machine.id === id)?.name ?? id;
  const selected = execution.selected_machine_id;
  const detail = execution.status === "unavailable"
    ? execution.machine_id
      ? "The selected Machine is offline or needs a compatible Plugin. No other Machine will be used."
      : "No online Machine has a compatible usage Plugin."
    : selected
    ? `Queries run on ${name(selected)}.`
    : "Queries run on this Service.";
  return (
    <Stack spacing={0.5} sx={{ mt: 1.5 }}>
      <TextField
        select
        size="small"
        label="Usage query Machine"
        value={execution.machine_id ?? ""}
        disabled={busy || me.role === "viewer"}
        error={error !== null || execution.status === "unavailable"}
        helperText={error ?? detail}
        onChange={(event): void => {
          const machine = event.target.value || null;
          setBusy(true);
          setError(null);
          void onChange(account, machine).catch((cause: unknown) => {
            setError(
              cause instanceof Error
                ? cause.message
                : "Could not update usage Machine",
            );
          }).finally(() => setBusy(false));
        }}
      >
        <MenuItem value="">Automatic</MenuItem>
        {execution.machine_id && !settings.machines.some((machine) =>
          machine.id === execution.machine_id
        ) && (
          <MenuItem value={execution.machine_id}>
            {execution.machine_id} (unavailable)
          </MenuItem>
        )}
        {settings.machines.map((machine) => (
          <MenuItem key={machine.id} value={machine.id}>
            {machine.name} · {machine.status}
          </MenuItem>
        ))}
      </TextField>
      {busy && (
        <Stack direction="row" spacing={1} alignItems="center" role="status">
          <CircularProgress size={desktopSize(14)} />
          <Typography variant="caption">
            Saving and refreshing usage…
          </Typography>
        </Stack>
      )}
      <Typography variant="caption" color="text.secondary">
        Applies to this account across Cowboy. Agent sessions keep their own
        Machine.
      </Typography>
    </Stack>
  );
}
