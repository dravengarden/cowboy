import { useId, useState } from "react";
import {
  Box,
  Button,
  FormControl,
  FormHelperText,
  InputLabel,
  LinearProgress,
  Select,
  Typography,
} from "@mui/material";
import { ExpandMore } from "@mui/icons-material";
import { SessionToolsSection } from "./AgentToolsPanel";
import { desktopSize } from "./surface/desktopSize";
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

// Provider categories are portable; ids cover older declarations without a category.
function isCoreOption(option: ConfigOption): boolean {
  return option.category === "model" || option.category === "thought_level" ||
    ["model", "effort", "reasoning_effort"].includes(option.id);
}

const compactSelect = {
  minHeight: desktopSize(44),
  maxWidth: "100%",
  borderRadius: 2,
  bgcolor: "action.hover",
  "& select": { py: 1, pl: 1.25, pr: "28px !important" },
  "&:focus-within": { outline: "2px solid", outlineColor: "primary.main" },
};

/** Core choices are direct native pickers; advanced forms mount only on request.
 * All edits use the settings sheet's signed options and acknowledged writers. */
export function SessionStartSettings(
  { sessionId }: { sessionId: string },
): React.JSX.Element | null {
  const id = useId();
  const [showOptions, setShowOptions] = useState(false);
  const [showTools, setShowTools] = useState(false);
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
  const core = options.filter(isCoreOption);
  const advanced = options.filter((option) => !isCoreOption(option));
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
    unavailable || runConfigPresetChanges(preset, options).some((change) => {
      const option = options.find((item) => item.id === change.configId);
      return !option || optionDisabled(option);
    });
  const renderOption = (
    option: ConfigOption,
    compact: boolean,
  ): React.JSX.Element => {
    const inputId = `${id}-${option.id}`;
    const current = String(option.currentValue);
    return (
      <FormControl
        key={option.id}
        fullWidth={!compact}
        size="small"
        sx={{ minWidth: 0, maxWidth: "100%" }}
      >
        {!compact && (
          <InputLabel htmlFor={inputId} shrink>{option.name}</InputLabel>
        )}
        <Select
          native
          variant={compact ? "standard" : "outlined"}
          {...(compact ? { disableUnderline: true } : { label: option.name })}
          value={current}
          disabled={optionDisabled(option) || option.options.length === 0}
          inputProps={{
            id: inputId,
            "aria-label": option.name,
            "data-start-option": option.id,
          }}
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
          sx={compact ? compactSelect : { minHeight: 44 }}
        >
          {!option.options.some((item) => String(item.value) === current) && (
            <option value={current} disabled>{current || "Not set"}</option>
          )}
          {option.options.map((item) => (
            <option key={String(item.value)} value={String(item.value)}>
              {compact ? `${option.name}: ${item.name}` : item.name}
            </option>
          ))}
        </Select>
        {!compact && option.description && (
          <FormHelperText>{option.description}</FormHelperText>
        )}
      </FormControl>
    );
  };

  return (
    <Box
      data-session-start-settings
      onClick={(event): void => event.stopPropagation()}
      onKeyDown={(event): void => event.stopPropagation()}
      sx={{
        width: "100%",
        maxWidth: 560,
        textAlign: "left",
        cursor: "default",
      }}
    >
      <Box
        data-start-core
        sx={{
          display: "flex",
          justifyContent: "center",
          flexWrap: "wrap",
          gap: 0.75,
        }}
      >
        {presets.length > 0 && (
          <FormControl size="small" sx={{ minWidth: 0, maxWidth: "100%" }}>
            <Select
              native
              variant="standard"
              disableUnderline
              value={active?.id ?? ""}
              disabled={unavailable}
              inputProps={{ "aria-label": "Preset", "data-start-preset": true }}
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
              sx={compactSelect}
            >
              <option value="" disabled>Preset: Custom</option>
              {presets.map((preset) => (
                <option
                  key={preset.id}
                  value={preset.id}
                  disabled={presetDisabled(preset)}
                >
                  Preset: {preset.name}
                </option>
              ))}
            </Select>
          </FormControl>
        )}
        {core.map((option) => renderOption(option, true))}
      </Box>
      <Box role="status" sx={{ textAlign: "center", position: "relative" }}>
        {(!connected || action.pending) && (
          <Typography variant="caption" color="text.secondary">
            {!connected ? "Reconnect to change settings." : "Saving settings…"}
          </Typography>
        )}
        {action.progress && <LinearProgress sx={{ mt: 0.5 }} />}
      </Box>
      <Box
        sx={{
          display: "flex",
          justifyContent: "center",
          flexWrap: "wrap",
          gap: 0.5,
          mt: 0.5,
        }}
      >
        {advanced.length > 0 && (
          <Button
            data-start-toggle="run"
            aria-keyshortcuts="Enter"
            aria-expanded={showOptions}
            aria-controls={`${id}-options`}
            onClick={(): void => setShowOptions(!showOptions)}
            endIcon={
              <ExpandMore
                sx={{ transform: showOptions ? "rotate(180deg)" : undefined }}
              />
            }
            sx={{
              minHeight: desktopSize(44),
              textTransform: "none",
              color: showOptions ? "primary.main" : "text.secondary",
            }}
          >
            More options
          </Button>
        )}
        <Button
          data-start-toggle="tools"
          aria-keyshortcuts="Enter"
          aria-expanded={showTools}
          aria-controls={`${id}-tools`}
          onClick={(): void => setShowTools(!showTools)}
          endIcon={
            <ExpandMore
              sx={{ transform: showTools ? "rotate(180deg)" : undefined }}
            />
          }
          sx={{
            minHeight: desktopSize(44),
            textTransform: "none",
            color: showTools ? "primary.main" : "text.secondary",
          }}
        >
          Tools & memory
        </Button>
      </Box>
      <Box id={`${id}-options`} hidden={!showOptions}>
        {showOptions && (
          <Box
            sx={{
              pt: 1.5,
              pb: 1,
              display: "grid",
              gridTemplateColumns:
                "repeat(auto-fit, minmax(min(100%, 250px), 1fr))",
              gap: 1.5,
            }}
          >
            {advanced.map((option) => renderOption(option, false))}
          </Box>
        )}
      </Box>
      <Box id={`${id}-tools`} hidden={!showTools}>
        {showTools && session && (
          <SessionToolsSection sessionId={sessionId} disabled={!connected} />
        )}
      </Box>
    </Box>
  );
}
