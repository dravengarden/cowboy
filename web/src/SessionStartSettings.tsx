import { useId } from "react";
import {
  Box,
  Divider,
  FormControl,
  FormHelperText,
  InputLabel,
  LinearProgress,
  Select,
  Typography,
} from "@mui/material";
import { SessionToolsSection } from "./AgentToolsPanel";
import { useNetworkActionState } from "./NetworkActionFeedback";
import { isManagedChild } from "./managedCalls";
import type { ConfigOption } from "./protocol";
import { useProviderCatalog } from "./providerCatalog";
import {
  providerConfigOptionDisabled,
  providerConfigOptionOrder,
  providerConfigOptionPresentations,
} from "./providerConfigOptions";
import {
  activeRunConfigPreset,
  runConfigPresetChanges,
  runConfigPresets,
} from "./runConfigPresets";
import { setSessionConfigOptions, useStoreSelector } from "./store";

const EMPTY_OPTIONS: readonly ConfigOption[] = [];

/** The empty conversation is a configuration workspace. Use the same signed
 * Provider options and acknowledged writers as the session settings sheet. */
export function SessionStartSettings(
  { sessionId }: { sessionId: string },
): React.JSX.Element | null {
  const id = useId();
  useProviderCatalog();
  const session = useStoreSelector((state) =>
    state.sessions.find((item) => item.id === sessionId)
  );
  const rawOptions = useStoreSelector((state) =>
    state.configOptions.get(sessionId) ?? EMPTY_OPTIONS
  );
  const connected = useStoreSelector((state) => state.connected);
  const action = useNetworkActionState();
  if (isManagedChild(session)) return null;
  const presentations = providerConfigOptionPresentations(
    session?.provider,
    session?.provider_version,
    session?.provider_generation_digest,
  );
  const options = [...rawOptions].sort((a, b) =>
    providerConfigOptionOrder(a, presentations.get(a.id)) -
      providerConfigOptionOrder(b, presentations.get(b.id)) ||
    a.id.localeCompare(b.id)
  );
  const presets = runConfigPresets(
    session?.provider,
    options,
    session?.provider_version,
    session?.provider_generation_digest,
  );
  const active = activeRunConfigPreset(presets, options);
  const unavailable = !session || !connected || action.pending;
  const optionDisabled = (option: ConfigOption): boolean =>
    unavailable ||
    providerConfigOptionDisabled(session!.status, presentations.get(option.id));
  const presetDisabled = (preset: (typeof presets)[number]): boolean =>
    unavailable ||
    runConfigPresetChanges(preset, options).some((change) => {
      const option = options.find((item) => item.id === change.configId);
      return !option || optionDisabled(option);
    });

  return (
    <Box
      data-session-start-settings
      onClick={(event): void => event.stopPropagation()}
      onKeyDown={(event): void => event.stopPropagation()}
      sx={{
        width: "100%",
        maxWidth: 720,
        textAlign: "left",
        cursor: "default",
      }}
    >
      <Box sx={{ position: "relative", pb: 1.5 }}>
        <Typography variant="subtitle2" color="text.primary">
          Run configuration
        </Typography>
        <Typography variant="caption" color="text.secondary" role="status">
          {!connected
            ? "Reconnect to change settings."
            : action.pending
            ? "Saving settings…"
            : "Changes save automatically."}
        </Typography>
        {action.progress && (
          <LinearProgress
            sx={{ position: "absolute", bottom: 4, left: 0, right: 0 }}
          />
        )}
      </Box>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns:
            "repeat(auto-fit, minmax(min(100%, 250px), 1fr))",
          gap: 1.5,
        }}
      >
        {presets.length > 0 && (
          <FormControl fullWidth size="small" sx={{ gridColumn: "1 / -1" }}>
            <InputLabel htmlFor={`${id}-preset`} shrink>Preset</InputLabel>
            <Select
              native
              label="Preset"
              value={active?.id ?? ""}
              disabled={unavailable}
              inputProps={{ id: `${id}-preset`, "data-start-preset": true }}
              onChange={(event): void => {
                const preset = presets.find((item) =>
                  item.id === event.target.value
                );
                if (!preset || presetDisabled(preset)) {
                  return;
                }
                void action.run(() =>
                  setSessionConfigOptions(
                    sessionId,
                    runConfigPresetChanges(preset, options),
                  )
                );
              }}
              sx={{ minHeight: 44 }}
            >
              <option value="" disabled>Custom configuration</option>
              {presets.map((preset) => (
                <option
                  key={preset.id}
                  value={preset.id}
                  disabled={presetDisabled(preset)}
                >
                  {preset.name}
                  {preset.isDefault ? " · Default" : ""}
                </option>
              ))}
            </Select>
            {active?.detail && <FormHelperText>{active.detail}</FormHelperText>}
          </FormControl>
        )}
        {options.map((option) => {
          const inputId = `${id}-${option.id}`;
          const current = String(option.currentValue);
          return (
            <FormControl key={option.id} fullWidth size="small">
              <InputLabel htmlFor={inputId} shrink>{option.name}</InputLabel>
              <Select
                native
                label={option.name}
                value={current}
                disabled={optionDisabled(option) || option.options.length === 0}
                inputProps={{ id: inputId, "data-start-option": option.id }}
                onChange={(event): void => {
                  const selected = option.options.find((item) =>
                    String(item.value) === event.target.value
                  );
                  if (!selected || optionDisabled(option)) return;
                  void action.run(() =>
                    setSessionConfigOptions(sessionId, [{
                      configId: option.id,
                      value: selected.value,
                    }])
                  );
                }}
                sx={{ minHeight: 44 }}
              >
                {!option.options.some((item) =>
                  String(item.value) === current
                ) && (
                  <option value={current} disabled>
                    {current || "Not set"}
                  </option>
                )}
                {option.options.map((item) => (
                  <option key={String(item.value)} value={String(item.value)}>
                    {item.name}
                  </option>
                ))}
              </Select>
              {option.description && (
                <FormHelperText>{option.description}</FormHelperText>
              )}
            </FormControl>
          );
        })}
      </Box>
      {options.length === 0 && (
        <Typography variant="body2" color="text.secondary">
          Provider options will appear here when available.
        </Typography>
      )}
      <Divider sx={{ mt: 2 }} />
      {session && (
        <SessionToolsSection sessionId={sessionId} disabled={!connected} />
      )}
    </Box>
  );
}
