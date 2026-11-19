import type { ProviderUiManifest } from "@cowboy/provider-ui";
import type { ConfigOption } from "./protocol";
import { currentProviderPresentationEntry } from "./providerCatalogRegistry";

export interface RunConfigPreset {
  id: string;
  name: string;
  detail: string;
  isDefault: boolean;
  values: Readonly<Record<string, string>>;
}

type DeclaredPreset = ProviderUiManifest["configuration"]["presets"][number];

/** Describe acknowledged values, including custom values outside the presets. */
export function runConfigSummary(options: readonly ConfigOption[]): string {
  return options.map((option) => {
    const value = String(option.currentValue);
    const label = option.options.find((choice) =>
      String(choice.value) === value
    )?.name ?? value;
    return `${option.name}: ${label}`;
  }).join(" · ");
}

/** Session-sheet title for the live configuration disclosure. */
export function runConfigCurrentTitle(presetName: string | undefined): string {
  return `Current · ${presetName ?? "Custom"}`;
}

/** Whether `preset` switches to another model the session advertises. */
function changesModel(
  preset: Pick<RunConfigPreset, "values">,
  optionById: ReadonlyMap<string, ConfigOption>,
): boolean {
  return Object.entries(preset.values).some(([id, value]) => {
    const option = optionById.get(id);
    return option?.category === "model" &&
      String(option.currentValue) !== value &&
      option.options.some((candidate) => String(candidate.value) === value);
  });
}

function supportsPreset(
  preset: Pick<RunConfigPreset, "values">,
  optionById: ReadonlyMap<string, ConfigOption>,
): boolean {
  const changingModel = changesModel(preset, optionById);
  return Object.entries(preset.values).every(([id, value]) => {
    const option = optionById.get(id);
    return option?.options.some((candidate) =>
        String(candidate.value) === value
      ) === true ||
      // Reasoning choices describe the CURRENT model, which may offer none
      // at all (a model-only preset such as Antigravity's Claude). A signed
      // preset is validated against its own target model: the serial config
      // path applies the model first, then that model validates its effort.
      (changingModel &&
        (option === undefined || option.category === "thought_level"));
  });
}

/** Whether applying `preset` switches the model first: its other values are
 *  then judged by the new model, even ones the current model does not show. */
export function runConfigPresetChangesModel(
  preset: Pick<RunConfigPreset, "values">,
  options: readonly ConfigOption[],
): boolean {
  return changesModel(
    preset,
    new Map(options.map((option) => [option.id, option])),
  );
}

/** Project signed, typed Provider presets onto the live configuration surface. */
export function supportedRunConfigPresets(
  declared: readonly DeclaredPreset[],
  options: readonly ConfigOption[],
): readonly RunConfigPreset[] {
  const optionById = new Map(options.map((option) => [option.id, option]));
  return declared.map((preset) => ({
    id: preset.id,
    name: preset.name,
    detail: preset.detail,
    isDefault: preset.is_default,
    values: { ...preset.values },
  })).filter((preset) => supportsPreset(preset, optionById));
}

export function runConfigPresets(
  provider: string | undefined,
  options: readonly ConfigOption[],
  providerVersion?: string | undefined,
  providerDigest?: string | undefined,
): readonly RunConfigPreset[] {
  if (!provider) return [];
  const declared =
    currentProviderPresentationEntry(provider, providerVersion, providerDigest)
      ?.manifest.configuration.presets ?? [];
  return supportedRunConfigPresets(declared, options);
}

export function activeRunConfigPreset(
  presets: readonly RunConfigPreset[],
  options: readonly ConfigOption[],
): RunConfigPreset | undefined {
  const optionById = new Map(options.map((option) => [option.id, option]));
  return presets.find((preset) =>
    Object.entries(preset.values).every(([id, value]) =>
      String(optionById.get(id)?.currentValue) === value
    )
  );
}

export function runConfigPresetChanges(
  preset: RunConfigPreset,
  options: readonly ConfigOption[],
): { configId: string; value: string }[] {
  const optionById = new Map(options.map((option) => [option.id, option]));
  return Object.entries(preset.values).flatMap(([configId, value]) =>
    String(optionById.get(configId)?.currentValue) === value
      ? []
      : [{ configId, value }]
  ).sort((left, right) =>
    Number(optionById.get(right.configId)?.category === "model") -
    Number(optionById.get(left.configId)?.category === "model")
  );
}

/** Whether applying `preset` needs an option `disabled` blocks. A value for
 *  an option the current model does not show is applied after the preset's
 *  model change, which then validates it. */
export function runConfigPresetBlocked(
  preset: RunConfigPreset,
  options: readonly ConfigOption[],
  disabled: (option: ConfigOption) => boolean,
): boolean {
  const changingModel = runConfigPresetChangesModel(preset, options);
  return runConfigPresetChanges(preset, options).some((change) => {
    const option = options.find((item) => item.id === change.configId);
    return option ? disabled(option) : !changingModel;
  });
}

/** Unsaved Run configuration edits, by option id. */
export type RunConfigDraft = Readonly<Record<string, string | boolean>>;

/** The options as the draft would leave them. */
export function withRunConfigDraft(
  options: readonly ConfigOption[],
  draft: RunConfigDraft,
): ConfigOption[] {
  return options.map((option) =>
    Object.hasOwn(draft, option.id)
      ? { ...option, currentValue: draft[option.id]! }
      : option
  );
}

/** The draft after one edit. Changing the model drops drafted values of
 *  options the current model does not show: they belonged to the model the
 *  draft switched to before, and the next preset brings its own. */
export function withRunConfigEdit(
  options: readonly ConfigOption[],
  draft: RunConfigDraft,
  id: string,
  value: string | boolean,
): RunConfigDraft {
  const option = options.find((candidate) => candidate.id === id);
  const kept = option?.category === "model"
    ? Object.fromEntries(
      Object.entries(draft).filter(([key]) =>
        options.some((candidate) => candidate.id === key)
      ),
    )
    : draft;
  return { ...kept, [id]: value };
}

/** What saving the draft sends: only values that differ from the agent's,
 *  the model first because effort choices depend on it. A drafted value for
 *  an option the current model does not show (a preset's reasoning level
 *  for the model it switches to) follows the model change. */
export function runConfigDraftChanges(
  options: readonly ConfigOption[],
  draft: RunConfigDraft,
): { configId: string; value: string | boolean }[] {
  const known = new Set(options.map((option) => option.id));
  const unshown = Object.entries(draft)
    .filter(([id]) => !known.has(id))
    .map(([configId, value]) => ({ configId, value, model: false }));
  const changes = options.flatMap((option) => {
    if (!Object.hasOwn(draft, option.id)) return [];
    const value = draft[option.id]!;
    if (String(value) === String(option.currentValue)) return [];
    const typed = option.options.find((candidate) =>
      String(candidate.value) === String(value)
    )?.value ?? value;
    return [{ configId: option.id, value: typed, model: option.category === "model" || option.id === "model" }];
  });
  return [...changes, ...unshown]
    .sort((left, right) => Number(right.model) - Number(left.model))
    .map(({ configId, value }) => ({ configId, value }));
}
