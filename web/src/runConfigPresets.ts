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

const EQUIVALENCE = "cowboy.dev/configuration-equivalence";

/** Whether the option already holds `value`: exactly, or through the
 * connected Provider's own exact equivalence (a resumed Fable 5.2 session
 * reports the alias its CLI lists). Mirrors the Controller's matcher. */
export function optionHolds(option: ConfigOption | undefined, value: string): boolean {
  if (!option) return false;
  const current = option.currentValue;
  if (String(current) === value) return true;
  const equivalence = option._meta?.[EQUIVALENCE] as
    | { schema?: unknown; value?: unknown; equivalent_values?: unknown }
    | undefined;
  return equivalence?.schema === 1 &&
    typeof current === "string" &&
    equivalence.value === current &&
    Array.isArray(equivalence.equivalent_values) &&
    equivalence.equivalent_values.length <= 8 &&
    equivalence.equivalent_values.every((item) => typeof item === "string") &&
    equivalence.equivalent_values.includes(value);
}

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

function supportsPreset(
  preset: Pick<RunConfigPreset, "values">,
  optionById: ReadonlyMap<string, ConfigOption>,
): boolean {
  const changingModel = Object.entries(preset.values).some(([id, value]) => {
    const option = optionById.get(id);
    return option?.category === "model" &&
      !optionHolds(option, value) &&
      option.options.some((candidate) => String(candidate.value) === value);
  });
  return Object.entries(preset.values).every(([id, value]) =>
    optionById.get(id)?.options.some((candidate) =>
        String(candidate.value) === value
      ) === true ||
    // A resumed session may list the held model under another exact
    // spelling; switching models still requires an advertised value.
    optionHolds(optionById.get(id), value) ||
    // Reasoning choices describe the CURRENT model. A signed preset for a
    // different advertised model must not inherit that model's restrictions.
    // The serial config path applies the model first, then validates its effort.
    (changingModel && optionById.get(id)?.category === "thought_level")
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
      optionHolds(optionById.get(id), value)
    )
  );
}

export function runConfigPresetChanges(
  preset: RunConfigPreset,
  options: readonly ConfigOption[],
): { configId: string; value: string }[] {
  const optionById = new Map(options.map((option) => [option.id, option]));
  return Object.entries(preset.values).flatMap(([configId, value]) =>
    optionHolds(optionById.get(configId), value)
      ? []
      : [{ configId, value }]
  ).sort((left, right) =>
    Number(optionById.get(right.configId)?.category === "model") -
    Number(optionById.get(left.configId)?.category === "model")
  );
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

/** The draft value for an edit. A value the live option already holds
 *  through the Provider's exact equivalence keeps the live spelling: a
 *  resumed session may not advertise the preset's spelling, so staging it
 *  would ask the Provider for a value it does not offer. */
export function runConfigDraftValue(
  options: readonly ConfigOption[],
  id: string,
  value: string | boolean,
): string | boolean {
  const live = options.find((option) => option.id === id);
  return typeof value === "string" && live && optionHolds(live, value)
    ? live.currentValue
    : value;
}

/** What saving the draft sends: only values that differ from the agent's,
 *  the model first because effort choices depend on it. */
export function runConfigDraftChanges(
  options: readonly ConfigOption[],
  draft: RunConfigDraft,
): { configId: string; value: string | boolean }[] {
  const changes = options.flatMap((option) => {
    if (!Object.hasOwn(draft, option.id)) return [];
    const value = draft[option.id]!;
    if (String(value) === String(option.currentValue)) return [];
    const typed = option.options.find((candidate) =>
      String(candidate.value) === String(value)
    )?.value ?? value;
    return [{ configId: option.id, value: typed, model: option.category === "model" || option.id === "model" }];
  });
  return changes
    .sort((left, right) => Number(right.model) - Number(left.model))
    .map(({ configId, value }) => ({ configId, value }));
}
