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
      const defaultCandidate = option.options.find((candidate) =>
        candidate.value === "default"
      );
      const representative = new Map<string, string | boolean>();
      for (const model of ["deepseek-v4-flash", "deepseek-v4-pro"]) {
        const candidates = option.options.filter((candidate) =>
          candidate.value !== "default" &&
          modelFor.get(String(candidate.value)) === model
        );
        const exact = candidates.find((candidate) =>
          String(candidate.value).replace(/\[1m\]$/, "") === model
        );
        const selected = exact ?? candidates[0];
        if (selected) representative.set(model, selected.value);
      }
      const currentModel = modelFor.get(String(option.currentValue));
      return {
        ...option,
        currentValue: option.currentValue === "default"
          ? "default"
          : currentModel
          ? representative.get(currentModel) ?? option.currentValue
          : option.currentValue,
        options: [
          ...(defaultCandidate
            ? [{ ...defaultCandidate, name: "Default · Flash (recommended)" }]
            : []),
          ...["deepseek-v4-flash", "deepseek-v4-pro"].flatMap((model) => {
            const value = representative.get(model);
            const candidate = option.options.find((item) =>
              item.value === value
            );
            return candidate ? [candidate] : [];
          }),
        ],
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
