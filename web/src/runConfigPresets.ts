import type { ConfigOption } from "./protocol";

export interface RunConfigPreset {
  id: "luna-max" | "sol-medium" | "sol-max" | "deepseek-flash-max";
  name: string;
  detail: string;
  isDefault: boolean;
  values: Readonly<Record<string, string>>;
}

const OPENAI_CODEX_PRESETS: readonly RunConfigPreset[] = [
  {
    id: "luna-max",
    name: "Luna · Max",
    detail: "GPT-5.6-Luna · Max reasoning",
    isDefault: true,
    values: { model: "gpt-5.6-luna", reasoning_effort: "max" },
  },
  {
    id: "sol-medium",
    name: "Sol · Medium",
    detail: "GPT-5.6-Sol · Medium reasoning",
    isDefault: false,
    values: { model: "gpt-5.6-sol", reasoning_effort: "medium" },
  },
  {
    id: "sol-max",
    name: "Sol · Max",
    detail: "GPT-5.6-Sol · Max reasoning",
    isDefault: false,
    values: { model: "gpt-5.6-sol", reasoning_effort: "max" },
  },
];

function supportsPreset(
  preset: RunConfigPreset,
  optionById: ReadonlyMap<string, ConfigOption>,
): boolean {
  return Object.entries(preset.values).every(([id, value]) =>
    optionById.get(id)?.options.some((candidate) =>
      String(candidate.value) === value
    ) === true
  );
}

/** Project provider-native recommended combinations from the live ACP surface. */
export function runConfigPresets(
  provider: string | undefined,
  options: readonly ConfigOption[],
): readonly RunConfigPreset[] {
  const optionById = new Map(options.map((option) => [option.id, option]));
  if (provider === "codex") {
    return OPENAI_CODEX_PRESETS.filter((preset) =>
      supportsPreset(preset, optionById)
    );
  }
  if (provider !== "claude-deepseek" && provider !== "codex-deepseek") {
    return [];
  }
  const model = optionById.get("model");
  const effortId = provider === "claude-deepseek"
    ? "effort"
    : "reasoning_effort";
  const flash = model?.options.find((candidate) =>
    String(candidate.value) !== "default" &&
    /deepseek-v4-flash/iu.test(`${String(candidate.value)} ${candidate.name}`)
  );
  if (!flash) return [];
  const preset: RunConfigPreset = {
    id: "deepseek-flash-max",
    name: "Flash · Max",
    detail: "DeepSeek-V4-Flash · Max reasoning",
    isDefault: true,
    values: { model: String(flash.value), [effortId]: "max" },
  };
  return supportsPreset(preset, optionById) ? [preset] : [];
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
  );
}
