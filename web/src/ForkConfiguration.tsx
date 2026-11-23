import { Alert, Box, Checkbox, FormControlLabel, LinearProgress, Stack, TextField } from "@mui/material";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import type { ConfigOption } from "./protocol";
import { SessionPresetPicker } from "./SessionPresetPicker";
import type { SessionPreset } from "./newSessionPreset";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { duplicateOptions } from "./sessionFork";
import { supportedRunConfigPresets } from "./runConfigPresets";
import { ConfigOptionControl, RecommendedPresetControls } from "./desktop/RunConfigControls";
import { currentOptionName } from "./desktop/runConfigOptions";
import { providerConfigOptionOrder } from "./providerConfigOptions";

export type ForkConfig = Record<string, string | boolean>;

/** The target agent's options before it exists, as the Controller previews
 *  them for this Provider, subscription, preset and inheritance: every
 *  option with what the new agent would start on as its current value.
 *  `fallback` (the source's live options) stands in for the same Provider
 *  while the preview loads or when it is unavailable. */
export function useForkTargetOptions(
  sessionId: string,
  target: { provider: string; account: string; preset: string; inherit: boolean },
  fallback: readonly ConfigOption[] | undefined,
  enabled: boolean,
): { options: readonly ConfigOption[]; loading: boolean; error: string } {
  const key = JSON.stringify([sessionId, target.provider, target.account, target.preset, target.inherit]);
  const [state, setState] = useState<{ key: string; options: ConfigOption[] | null; error: string }>({
    key: "",
    options: null,
    error: "",
  });
  useEffect(() => {
    if (!enabled || !target.provider || !target.account) return undefined;
    const abort = new AbortController();
    void duplicateOptions(
      sessionId,
      {
        provider: target.provider,
        provider_account_id: target.account,
        preset: target.preset || null,
        inherit_settings: target.inherit,
      },
      abort.signal,
    ).then(
      (options) => setState({ key, options, error: "" }),
      (cause: unknown) => {
        if (abort.signal.aborted) return;
        setState({
          key,
          options: null,
          error: cause instanceof Error ? cause.message : "Options are unavailable",
        });
      },
    );
    return () => abort.abort();
  }, [key, enabled]);
  const current = state.key === key ? state : null;
  const options = current?.options ?? fallback ?? [];
  return {
    options,
    loading: enabled && current === null,
    error: current?.error ?? "",
  };
}

/** Draft choices for the new agent; never writes the source's live options.
 *  `options` are the target's, current values being what it would start on;
 *  `overrides` change them for this Replace or Fork. */
export function ForkConfiguration({
  provider,
  presets,
  preset,
  onPreset,
  inherit,
  onInherit,
  options,
  loading,
  optionsError,
  overrides,
  onOverrides,
  advanced,
  locked,
  project,
  disclosure,
  sameProvider,
}: {
  provider: string;
  presets: readonly SessionPreset[];
  preset: string;
  onPreset: (value: string) => void;
  inherit: boolean;
  onInherit: (value: boolean) => void;
  options: readonly ConfigOption[];
  loading: boolean;
  optionsError: string;
  overrides: ForkConfig;
  onOverrides: (value: ForkConfig) => void;
  advanced: boolean;
  locked: boolean;
  project: ReactNode;
  disclosure: ReactNode;
  sameProvider: boolean;
}): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const ordered = useMemo(
    () =>
      [...options].sort(
        (left, right) =>
          providerConfigOptionOrder(left, undefined) - providerConfigOptionOrder(right, undefined),
      ),
    [options],
  );
  const effective = useMemo(
    () =>
      ordered.map((option) =>
        Object.hasOwn(overrides, option.id)
          ? { ...option, currentValue: overrides[option.id]! }
          : option,
      ),
    [ordered, overrides],
  );
  const edit = (id: string, value: string | boolean): void => {
    const base = ordered.find((option) => option.id === id);
    const next = { ...overrides };
    if (base && String(base.currentValue) === String(value)) delete next[id];
    else next[id] = value;
    onOverrides(next);
  };
  const inheritToggle = (
    <FormControlLabel
      data-fork-inherit
      sx={{ mr: 0 }}
      control={
        <Checkbox
          size={desktop ? "small" : "medium"}
          checked={inherit}
          disabled={locked}
          onChange={(_, checked) => onInherit(checked)}
        />
      }
      label="Inherit compatible settings"
      slotProps={desktop ? { typography: { variant: "caption", fontWeight: 750, color: "text.secondary" } } : {}}
    />
  );
  const status = (
    <>
      {loading && <LinearProgress aria-label="Loading the new agent's options" sx={{ borderRadius: 1 }} />}
      {optionsError && (
        <Alert severity="warning" data-fork-options-error>
          {optionsError}
        </Alert>
      )}
    </>
  );
  if (desktop) {
    return (
      <DesktopForkFields
        presets={presets}
        ordered={ordered}
        effective={effective}
        overrides={overrides}
        locked={locked}
        onEdit={edit}
      >
        {status}
        <Box data-fork-project>{project}</Box>
        {inheritToggle}
      </DesktopForkFields>
    );
  }
  return (
    <TouchForkFields
      provider={provider}
      presets={presets}
      preset={preset}
      onPreset={onPreset}
      defaultLabel={inherit && sameProvider ? "Inherit current model & reasoning" : "Agent default"}
      ordered={ordered}
      overrides={overrides}
      onOverrides={onOverrides}
      onEdit={edit}
      locked={locked}
      advanced={advanced}
      disclosure={disclosure}
    >
      {status}
      {project}
      {inheritToggle}
    </TouchForkFields>
  );
}

/** Desktop: the Agent pane's grammar. Presets fill the options (`1`–`9`),
 *  each option is one field (`J/K`, `H/L`, its letter), and a changed field
 *  names what the new agent would otherwise start on. `children` are the
 *  status, the project (where, before how) and the inheritance control. */
function DesktopForkFields({
  presets,
  ordered,
  effective,
  overrides,
  locked,
  onEdit,
  children,
}: {
  presets: readonly SessionPreset[];
  ordered: readonly ConfigOption[];
  effective: readonly ConfigOption[];
  overrides: ForkConfig;
  locked: boolean;
  onEdit: (id: string, value: string | boolean) => void;
  children: [ReactNode, ReactNode, ReactNode];
}): React.JSX.Element {
  const [status, project, inherit] = children;
  const cards = supportedRunConfigPresets(presets, effective);
  return (
    <Stack spacing={1} data-fork-configuration>
      {status}
      {project}
      {cards.length > 0 && <RecommendedPresetControls presets={cards} options={effective} onChange={onEdit} />}
      {effective.map((option) => {
        const base = ordered.find((entry) => entry.id === option.id) ?? option;
        return (
          <ConfigOptionControl
            key={option.id}
            option={option}
            disabled={locked}
            savedName={Object.hasOwn(overrides, option.id) ? currentOptionName(base) : undefined}
            savedTitle={(name) => `Changed · the new agent would otherwise start on ${name}`}
            onChange={onEdit}
          />
        );
      })}
      {inherit}
    </Stack>
  );
}

/** Touch keeps the platform pickers: a preset, then the core options, the
 *  rest behind More options. `children`: status, project, inheritance. */
function TouchForkFields({
  provider,
  presets,
  preset,
  onPreset,
  defaultLabel,
  ordered,
  overrides,
  onOverrides,
  onEdit,
  locked,
  advanced,
  disclosure,
  children,
}: {
  provider: string;
  presets: readonly SessionPreset[];
  preset: string;
  onPreset: (value: string) => void;
  defaultLabel: string;
  ordered: readonly ConfigOption[];
  overrides: ForkConfig;
  onOverrides: (value: ForkConfig) => void;
  onEdit: (id: string, value: string | boolean) => void;
  locked: boolean;
  advanced: boolean;
  disclosure: ReactNode;
  children: [ReactNode, ReactNode, ReactNode];
}): React.JSX.Element {
  const [status, project, inherit] = children;
  const optionRow = (option: ConfigOption) => {
    const base = ordered.find((entry) => entry.id === option.id) ?? option;
    const current = currentOptionName(base);
    return (
      <TextField
        key={option.id}
        select
        label={option.name}
        value={overrides[option.id] === undefined ? "" : String(overrides[option.id])}
        disabled={locked}
        slotProps={{ select: { native: true, displayEmpty: true }, inputLabel: { shrink: true } }}
        helperText={option.description}
        onChange={(event) => {
          if (!event.target.value) {
            const next = { ...overrides };
            delete next[option.id];
            onOverrides(next);
            return;
          }
          const choice = option.options.find((item) => String(item.value) === event.target.value);
          if (choice) onEdit(option.id, choice.value);
        }}
        inputProps={{ "data-fork-option": option.id }}
      >
        <option value="">Default: {current}</option>
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
        defaultLabel={defaultLabel}
        helperText=""
      />
      {status}
      {ordered.filter(core).map(optionRow)}
      {disclosure}
      {advanced && (
        <Stack spacing={1.5} data-context-advanced>
          {project}
          {inherit}
          {ordered.filter((option) => !core(option)).map(optionRow)}
        </Stack>
      )}
    </Stack>
  );
}
