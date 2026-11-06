import { MenuItem, TextField } from "@mui/material";
import type { SessionPreset } from "./newSessionPreset";

export function SessionPresetPicker({
  provider,
  presets,
  value,
  onChange,
  helperText,
  defaultLabel = "Agent default",
  disabled = false,
}: {
  provider: string;
  presets: readonly SessionPreset[];
  value: string;
  onChange: (value: string) => void;
  helperText: string;
  defaultLabel?: string;
  disabled?: boolean;
}): React.JSX.Element {
  return (
    <TextField
      select
      label="Model and reasoning preset"
      disabled={disabled}
      value={value}
      slotProps={{
        select: { displayEmpty: true },
        inputLabel: { shrink: true },
      }}
      onChange={(event) => onChange(event.target.value)}
      helperText={helperText}
    >
      <MenuItem value="">{defaultLabel}</MenuItem>
      {value && !presets.some((preset) => `${provider}/${preset.id}` === value) && (
        <MenuItem value={value}>{value} · Unavailable or incompatible</MenuItem>
      )}
      {presets.map((preset) => (
        <MenuItem key={preset.id} value={`${provider}/${preset.id}`}>
          {preset.name}
          {preset.is_default ? " · Recommended" : ""} · {preset.detail}
        </MenuItem>
      ))}
    </TextField>
  );
}
