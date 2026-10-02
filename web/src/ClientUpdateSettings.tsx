import { MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { setClientUpdateDelay, setClientUpdateMode, useClientUpdateSettings } from "./clientUpdateSettings";

export function ClientUpdateSettings() {
  const settings = useClientUpdateSettings();
  const [delayInput, setDelayInput] = useState(String(settings.countdownSecs));
  useEffect(() => setDelayInput(String(settings.countdownSecs)), [settings.countdownSecs]);
  return (
    <Stack spacing={1.5} sx={{ p: 1.5 }}>
      <Typography variant="subtitle2">Web updates</Typography>
      <TextField select label="Apply downloaded updates" size="small" value={settings.mode}
        onChange={(event) => setClientUpdateMode(event.target.value === "manual" ? "manual" : "automatic")}>
        <MenuItem value="automatic">Automatically after countdown</MenuItem>
        <MenuItem value="manual">When I click Update</MenuItem>
      </TextField>
      {settings.mode === "automatic" && (
        <TextField label="Countdown (seconds)" type="number" size="small" value={delayInput}
          slotProps={{ htmlInput: { min: 0, max: 3600, step: 1 } }}
          onChange={(event) => {
            setDelayInput(event.target.value);
            const value = Number(event.target.value);
            if (event.target.value !== "" && Number.isInteger(value) && value >= 0 && value <= 3600) setClientUpdateDelay(value);
          }}
          onBlur={() => setDelayInput(String(settings.countdownSecs))}
          helperText="0 applies immediately after download. Default: 3 seconds." />
      )}
      <Typography variant="caption" color="text.secondary">Saved on this device. Updates download in the background in either mode.</Typography>
    </Stack>
  );
}
