import { Alert, Checkbox, FormControlLabel, Stack, TextField } from "@mui/material";
import type { ReactNode } from "react";
import type { ConfigOption } from "./protocol";
import { SessionPresetPicker } from "./SessionPresetPicker";
import type { SessionPreset } from "./newSessionPreset";

export type ForkConfig = Record<string, string | boolean>;

/** Draft choices for the new copy; never writes the source's live options. */
export function ForkConfiguration({
  provider,
  presets,
  preset,
  onPreset,
  inherit,
  onInherit,
  sameProvider,
  options,
  overrides,
  onOverrides,
  advanced,
  locked,
  project,
  disclosure,
}: {
  provider: string;
  presets: readonly SessionPreset[];
  preset: string;
  onPreset: (value: string) => void;
  inherit: boolean;
  onInherit: (value: boolean) => void;
  sameProvider: boolean;
  options: readonly ConfigOption[];
  overrides: ForkConfig;
  onOverrides: (value: ForkConfig) => void;
  advanced: boolean;
  locked: boolean;
  project: ReactNode;
  disclosure: ReactNode;
}): React.JSX.Element {
  const selectedPreset = presets.find((item) => `${provider}/${item.id}` === preset);
  const optionRow = (option: ConfigOption) => {
    const inherited = inherit ? String(option.currentValue) : "Agent default";
    const base = selectedPreset?.values[option.id] ?? inherited;
    const current = option.options.find((item) => String(item.value) === base)?.name ?? base;
    return (
      <TextField
        key={option.id}
        select
        label={option.name}
        value={overrides[option.id] === undefined ? "" : String(overrides[option.id])}
        disabled={locked}
        slotProps={{ select: { native: true }, inputLabel: { shrink: true } }}
        helperText={option.description}
        onChange={(event) => {
          const next = { ...overrides };
          if (!event.target.value) delete next[option.id];
          else {
            const choice = option.options.find((item) => String(item.value) === event.target.value);
            if (!choice) return;
            next[option.id] = choice.value;
          }
          onOverrides(next);
        }}
        inputProps={{ "data-fork-option": option.id }}
      >
        <option value="">
          {inherit && !selectedPreset ? "Inherit" : "Default"}: {current}
        </option>
        {option.options.map((item) => (
          <option key={String(item.value)} value={String(item.value)}>
            {item.name}
          </option>
        ))}
      </TextField>
    );
  };
  const core = (option: ConfigOption) =>
    option.category === "model" ||
    option.category === "thought_level" ||
    ["model", "effort", "reasoning_effort"].includes(option.id);
  return (
    <Stack spacing={1.5} data-fork-configuration>
      <SessionPresetPicker
        provider={provider}
        presets={presets}
        value={preset}
        disabled={locked}
        onChange={(value) => {
          onPreset(value);
          onOverrides({});
        }}
        defaultLabel={
          inherit && sameProvider ? "Inherit current model & reasoning" : "Agent default"
        }
        helperText={
          presets.find((item) => item.is_default)
            ? `Recommended: ${presets.find((item) => item.is_default)!.detail}`
            : ""
        }
      />
      {sameProvider && options.filter(core).map(optionRow)}
      {disclosure}
      {advanced && (
        <Stack spacing={1.5} data-context-advanced>
          {project}
          <FormControlLabel
            sx={{ mr: 0 }}
            control={
              <Checkbox
                checked={inherit}
                disabled={locked}
                onChange={(_, checked) => onInherit(checked)}
              />
            }
            label="Inherit compatible settings"
          />
          {sameProvider && options.filter((option) => !core(option)).map(optionRow)}
          {!sameProvider && (
            <Alert severity="info">
              Choose a target Agent preset above. Individual options are available when forking with
              the current Provider.
            </Alert>
          )}
        </Stack>
      )}
      {Object.keys(overrides).length > 0 && (
        <Alert severity="info">
          Individual options apply to Fork only. Clear them to use Replace.
        </Alert>
      )}
    </Stack>
  );
}
