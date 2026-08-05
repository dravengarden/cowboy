import type { ConfigOption } from "./protocol";

// Claude Code exposes tier aliases and a custom model as separate ACP rows,
// even when a gateway maps several rows to the same upstream model. Present
// the provider contract: one row per real model and distinct effort behavior.
export function providerConfigOptions(
  provider: string | undefined,
  options: readonly ConfigOption[],
): ConfigOption[] {
  if (provider !== "claude-deepseek") return [...options];
  return options.map((option) => {
    if (option.id === "model") {
      const modelFor = new Map(
        option.options.map((candidate) => [
          String(candidate.value),
          candidate.value === "default"
            ? "deepseek-v4-flash"
            : candidate.name.replace(/\[1m\]$/, ""),
        ]),
      );
      const representative = new Map<string, string | boolean>();
      representative.set("deepseek-v4-flash", "default");
      for (const candidate of option.options) {
        const model = modelFor.get(String(candidate.value));
        if (model && !representative.has(model)) {
          representative.set(model, candidate.value);
        }
      }
      const currentModel = modelFor.get(String(option.currentValue));
      return {
        ...option,
        currentValue: currentModel
          ? representative.get(currentModel) ?? option.currentValue
          : option.currentValue,
        options: option.options.filter((candidate) =>
          representative.get(modelFor.get(String(candidate.value)) ?? "") ===
            candidate.value
        ),
      };
    }
    if (option.id === "effort" || option.id === "reasoning_effort") {
      return {
        ...option,
        name: "Reasoning effort",
        description:
          "DeepSeek automatic, high, or maximum reasoning for this session",
        options: option.options
          .filter((candidate) =>
            ["default", "high", "max"].includes(String(candidate.value))
          )
          .map((candidate) =>
            candidate.value === "default"
              ? { ...candidate, name: "Automatic (recommended)" }
              : candidate
          ),
      };
    }
    return option;
  });
}
