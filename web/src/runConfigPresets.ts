import type { ProviderUiManifest } from "@cowboy/provider-ui";
import type { ConfigOption } from "./protocol";
import { currentProviderEntry } from "./providerCatalogRegistry";

export interface RunConfigPreset {
  id: string;
  name: string;
  detail: string;
  isDefault: boolean;
  values: Readonly<Record<string, string>>;
}

type DeclaredPreset = ProviderUiManifest["configuration"]["presets"][number];

function supportsPreset(
  preset: Pick<RunConfigPreset, "values">,
  optionById: ReadonlyMap<string, ConfigOption>,
): boolean {
  const changingModel = Object.entries(preset.values).some(([id, value]) => {
    const option = optionById.get(id);
    return option?.category === "model" &&
      String(option.currentValue) !== value &&
      option.options.some((candidate) => String(candidate.value) === value);
  });
  return Object.entries(preset.values).every(([id, value]) =>
    optionById.get(id)?.options.some((candidate) =>
        String(candidate.value) === value
      ) === true ||
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
    currentProviderEntry(provider, providerVersion, providerDigest)
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
