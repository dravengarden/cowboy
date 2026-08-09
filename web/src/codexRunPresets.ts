import type { ConfigOption } from "./protocol";

export interface CodexRunPreset {
  id: "luna-max" | "sol-medium";
  name: string;
  detail: string;
  isDefault: boolean;
  values: Readonly<Record<"model" | "reasoning_effort", string>>;
}

const CODEX_RUN_PRESETS: readonly CodexRunPreset[] = [
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
];

/** Only expose presets that the live OpenAI Codex ACP surface can apply. */
export function codexRunPresets(
  provider: string | undefined,
  options: readonly ConfigOption[],
): readonly CodexRunPreset[] {
  if (provider !== "codex") return [];
  const optionById = new Map(options.map((option) => [option.id, option]));
  return CODEX_RUN_PRESETS.filter((preset) =>
    Object.entries(preset.values).every(([id, value]) =>
      optionById.get(id)?.options.some((candidate) =>
        String(candidate.value) === value
      ) === true
    )
  );
}

export function activeCodexRunPreset(
  presets: readonly CodexRunPreset[],
  options: readonly ConfigOption[],
): CodexRunPreset | undefined {
  const optionById = new Map(options.map((option) => [option.id, option]));
  return presets.find((preset) =>
    Object.entries(preset.values).every(([id, value]) =>
      String(optionById.get(id)?.currentValue) === value
    )
  );
}

export function codexRunPresetChanges(
  preset: CodexRunPreset,
  options: readonly ConfigOption[],
): { configId: string; value: string }[] {
  const optionById = new Map(options.map((option) => [option.id, option]));
  return Object.entries(preset.values).flatMap(([configId, value]) =>
    String(optionById.get(configId)?.currentValue) === value
      ? []
      : [{ configId, value }]
  );
}
