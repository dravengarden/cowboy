import { type HTMLAttributes, useEffect, useRef } from "react";
import {
  Box,
  ButtonBase,
  FormControl,
  MenuItem,
  Select,
  Stack,
  type SxProps,
  type Theme,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import { desktopSize } from "../surface/desktopSize";
import { Kbd } from "../Kbd";
import { HintTooltip } from "../HintTooltip";
import type { ConfigOption } from "../protocol";
import type { ProviderConfigOptionPresentation } from "../providerConfigOptions";
import {
  activeRunConfigPreset,
  type RunConfigPreset,
  runConfigPresetChanges,
} from "../runConfigPresets";
import { nextRunConfigChoiceIndex } from "./runConfigKeyboard";
import { RUN_CONFIG_STEP_EVENT } from "./runConfigNavigation";
import { optionLabel, optionShortcut } from "./runConfigOptions";

// The Agent pane's fields: one row per Provider option and the recommended
// presets. They edit the Run configuration draft, never the agent itself.

/** The Model picker: a compact outlined Select in the option grid. */
const modelSelectSx: SxProps<Theme> = {
  height: desktopSize(34),
  borderRadius: 1.25,
  fontSize: "0.75rem",
  fontWeight: 650,
  "& .MuiSelect-select": { px: 1.2, py: 0.75 },
  bgcolor: (theme) => alpha(theme.palette.background.default, 0.42),
  "& .MuiOutlinedInput-notchedOutline": {
    borderColor: (theme) => alpha(theme.palette.divider, 0.78),
  },
  "&:hover .MuiOutlinedInput-notchedOutline": {
    borderColor: (theme) => alpha(theme.palette.primary.main, 0.42),
  },
  "&.Mui-focused .MuiOutlinedInput-notchedOutline": {
    borderWidth: 1,
    borderColor: "primary.main",
  },
};

/** A Provider option's choices as one segmented pill. */
const choiceGroupSx: SxProps<Theme> = {
  display: "flex",
  flexWrap: "nowrap",
  gap: 0.5,
  p: 0.5,
  borderRadius: 1.5,
  bgcolor: (theme) => alpha(theme.palette.background.default, 0.48),
  border: 1,
  borderColor: (theme) => alpha(theme.palette.divider, 0.62),
  "& .MuiToggleButtonGroup-grouped": {
    borderRadius: 1.1,
    border: 0,
    color: "text.secondary",
    "&.Mui-selected": {
      color: "primary.main",
      bgcolor: (theme) => alpha(theme.palette.primary.main, 0.14),
      boxShadow: (theme) =>
        `inset 0 0 0 1px ${alpha(theme.palette.primary.main, 0.28)}`,
    },
    "&.Mui-selected:hover": {
      bgcolor: (theme) => alpha(theme.palette.primary.main, 0.18),
    },
  },
};

// Size each segment from its own label, then share the spare
// width. Equal `1 1 0` segments with `nowrap` let a long
// Provider label ("Bypass permissions") paint past its pill.
// When the row is genuinely too narrow a label wraps at a word
// instead; the segment never shrinks below its longest word.
const choiceButtonSx: SxProps<Theme> = {
  minHeight: desktopSize(28),
  px: 1,
  py: 0.2,
  flex: "1 1 auto",
  fontSize: "0.6875rem",
  lineHeight: 1.15,
  textTransform: "none",
  whiteSpace: "normal",
  textAlign: "center",
};

/** "was Opus 5.5": the agent's value under a field the draft changes. */
function UnsavedFrom({ name }: { name: string }): React.JSX.Element {
  return (
    <Typography
      variant="caption"
      color="warning.main"
      noWrap
      title={`Unsaved · the agent runs ${name}`}
      sx={{ display: "block", fontSize: "0.625rem", fontWeight: 650 }}
    >
      was {name}
    </Typography>
  );
}

export function ConfigOptionControl({
  option,
  disabled,
  savedName,
  onChange,
}: {
  /** The option as the unsaved draft leaves it. */
  option: ConfigOption;
  presentation?: ProviderConfigOptionPresentation | undefined;
  disabled: boolean;
  /** The agent's value when the draft changes it. */
  savedName?: string | undefined;
  onChange: (id: string, value: string | boolean) => void;
}): React.JSX.Element {
  const label = optionLabel(option);
  const shortcut = optionShortcut(option);
  const rowRef = useRef<HTMLDivElement>(null);
  const setValue = (value: string): void => {
    const selected = option.options.find((candidate) =>
      String(candidate.value) === value
    );
    if (!selected) return;
    onChange(option.id, selected.value);
  };
  // `H/L` on a Select steps its value in place; `Enter` or the field's
  // letter opens the full list.
  const stepRef = useRef<(delta: -1 | 1) => void>(() => undefined);
  stepRef.current = (delta): void => {
    if (disabled) return;
    const current = option.options.findIndex((candidate) =>
      String(candidate.value) === String(option.currentValue)
    );
    const next = nextRunConfigChoiceIndex(
      option.options.length,
      current,
      delta,
      false,
    );
    const choice = option.options[next];
    if (choice && next !== current) setValue(String(choice.value));
  };
  useEffect(() => {
    const row = rowRef.current;
    if (!row || label !== "Model") return undefined;
    const step = (event: Event): void => {
      event.preventDefault();
      stepRef.current((event as CustomEvent<{ delta: -1 | 1 }>).detail.delta);
    };
    row.addEventListener(RUN_CONFIG_STEP_EVENT, step);
    return (): void => row.removeEventListener(RUN_CONFIG_STEP_EVENT, step);
  }, [label]);
  return (
    <Box
      ref={rowRef}
      data-config-row
      data-config-shortcut={shortcut?.toLowerCase()}
      data-config-changed={savedName !== undefined || undefined}
      sx={{
        minWidth: 0,
        display: "grid",
        gridTemplateColumns: "120px minmax(0, 1fr)",
        alignItems: "center",
        gap: 1.25,
      }}
    >
      <Box sx={{ minWidth: 0 }}>
        <Tooltip title={option.description ?? ""} placement="right">
          <Stack direction="row" alignItems="center" spacing={0.5}>
            <Typography
              variant="caption"
              fontWeight={750}
              color="text.secondary"
              sx={{
                cursor: option.description ? "help" : "default",
                letterSpacing: "0.02em",
              }}
            >
              {label}
            </Typography>
            {shortcut && (
              <Kbd
                keys={shortcut}
                availability={disabled ? "inactive" : "available"}
              />
            )}
          </Stack>
        </Tooltip>
        {savedName !== undefined && <UnsavedFrom name={savedName} />}
      </Box>
      {label === "Model"
        ? (
          <FormControl fullWidth size="small">
            <Select
              disabled={disabled}
              value={String(option.currentValue)}
              SelectDisplayProps={{
                "data-config-choice": "true",
                "data-config-select": "true",
              } as HTMLAttributes<HTMLDivElement>}
              onChange={(event): void => setValue(String(event.target.value))}
              aria-label="Model"
              sx={modelSelectSx}
            >
              {option.options.map((candidate) => (
                <MenuItem
                  key={String(candidate.value)}
                  value={String(candidate.value)}
                  dense
                >
                  {candidate.name}
                </MenuItem>
              ))}
            </Select>
          </FormControl>
        )
        : (
          <ToggleButtonGroup
            disabled={disabled}
            exclusive
            size="small"
            value={String(option.currentValue)}
            onChange={(_event, value: string | null): void => {
              if (value !== null) setValue(value);
            }}
            sx={choiceGroupSx}
          >
            {option.options.map((candidate) => (
              <ToggleButton
                data-config-choice
                key={String(candidate.value)}
                value={String(candidate.value)}
                sx={choiceButtonSx}
              >
                {candidate.name}
              </ToggleButton>
            ))}
          </ToggleButtonGroup>
        )}
    </Box>
  );
}

export function RecommendedPresetControls({
  presets,
  options,
  onChange,
}: {
  presets: readonly RunConfigPreset[];
  /** The options as the unsaved draft leaves them. */
  options: readonly ConfigOption[];
  onChange: (id: string, value: string | boolean) => void;
}): React.JSX.Element {
  const active = activeRunConfigPreset(presets, options);
  return (
    <Stack data-config-row spacing={0.75} sx={{ minWidth: 0 }}>
      <Typography
        variant="caption"
        fontWeight={750}
        color="text.secondary"
        sx={{ letterSpacing: "0.02em" }}
      >
        Recommended
      </Typography>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(168px, 1fr))",
          gap: 0.75,
        }}
      >
        {presets.map((preset, index) => {
          const selected = active?.id === preset.id;
          return (
            <ButtonBase
              key={preset.id}
              data-config-preset={index}
              data-config-choice
              aria-pressed={selected}
              onClick={(): void => {
                for (const change of runConfigPresetChanges(preset, options)) {
                  onChange(change.configId, change.value);
                }
              }}
              sx={{
                minWidth: 0,
                minHeight: desktopSize(44),
                px: 1.2,
                py: 0.65,
                borderRadius: 1.25,
                border: 1,
                borderColor: selected ? "primary.main" : "divider",
                bgcolor: (theme) =>
                  selected
                    ? alpha(theme.palette.primary.main, 0.14)
                    : alpha(theme.palette.background.default, 0.42),
                justifyContent: "flex-start",
                textAlign: "left",
                "&:hover": { bgcolor: "action.hover" },
                "&.Mui-disabled": { opacity: 0.46 },
              }}
            >
              <Box sx={{ flex: 1, minWidth: 0 }}>
                <Stack
                  direction="row"
                  spacing={0.65}
                  alignItems="center"
                  sx={{ minWidth: 0 }}
                >
                  <HintTooltip title={preset.name}>
                    <Typography
                      variant="caption"
                      fontWeight={750}
                      color={selected ? "primary.main" : "text.primary"}
                      noWrap
                      sx={{ minWidth: 0 }}
                    >
                      {preset.name}
                    </Typography>
                  </HintTooltip>
                  {preset.isDefault && (
                    <Typography
                      variant="caption"
                      color="primary.main"
                      sx={{
                        flexShrink: 0,
                        fontSize: "0.625rem",
                        fontWeight: 750,
                      }}
                    >
                      Default
                    </Typography>
                  )}
                </Stack>
                <HintTooltip title={preset.detail}>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    noWrap
                    sx={{ display: "block", fontSize: "0.625rem" }}
                  >
                    {preset.detail}
                  </Typography>
                </HintTooltip>
              </Box>
              <Kbd
                keys={String(index + 1)}
                availability="available"
              />
            </ButtonBase>
          );
        })}
      </Box>
    </Stack>
  );
}
