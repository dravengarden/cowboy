import { MenuItem, TextField } from "@mui/material";
import type { SessionPreset } from "./newSessionPreset";

export function SessionPresetPicker(
  { provider, presets, value, onChange, helperText }: {
    provider: string;
    presets: readonly SessionPreset[];
    value: string;
    onChange: (value: string) => void;
    helperText: string;
  },
): React.JSX.Element {
  return (
    <TextField
      select
      label="Model and reasoning preset"
      value={value}
      slotProps={{
        select: { displayEmpty: true },
        inputLabel: { shrink: true },
      }}
      onChange={(event) => onChange(event.target.value)}
      helperText={helperText}
    >
      <MenuItem value="">Agent default</MenuItem>
      {value &&
        !presets.some((preset) => `${provider}/${preset.id}` === value) &&
        <MenuItem value={value}>{value} · Unavailable or incompatible
        </MenuItem>}
      {presets.map((preset) => (
        <MenuItem key={preset.id} value={`${provider}/${preset.id}`}>
          {preset.name} · {preset.detail}
        </MenuItem>
      ))}
    </TextField>
  );
}
