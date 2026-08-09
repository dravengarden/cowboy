import type { ConfigOption } from "./protocol";

const DEEPSEEK_CONTEXT_WINDOWS: Record<string, string> = {
  "128k": "128K",
  "256k": "256K",
  "512k": "512K",
  "680k": "680K",
  "830k": "830K",
};

const DEEPSEEK_COMPACTION_POINTS: Record<string, Record<string, string>> = {
  "claude-deepseek": {
    "128k": "128K",
    "256k": "256K",
    "512k": "512K",
    "680k": "680K",
    "830k": "819.2K",
  },
  "codex-deepseek": {
    "128k": "121.6K",
    "256k": "243.2K",
    "512k": "486.4K",
    "680k": "646K",
    "830k": "788.5K",
  },
};

function deepseekContextOption(
  provider: string,
  option: ConfigOption,
): ConfigOption {
  const compactionPoints = DEEPSEEK_COMPACTION_POINTS[provider];
  return {
    ...option,
    name: "Working context",
    options: option.options.map((candidate) => {
      const value = String(candidate.value);
      const window = DEEPSEEK_CONTEXT_WINDOWS[value];
      const compaction = compactionPoints?.[value];
      if (!window || !compaction) return candidate;
      const suffix = provider === "codex-deepseek" && value === "830k"
        ? " · large"
        : "";
      return {
        ...candidate,
        name: `${window} window · compacts at ${compaction}${suffix}`,
      };
    }),
  };
}

function withoutRecommendation(name: string): string {
  return name
    .replace(/\s*[·(]\s*recommended\)?$/iu, "")
    .trim();
}

export function currentConfigOptionName(
  option: ConfigOption | undefined,
): string | null {
  if (!option) return null;
  return option.options.find((candidate) =>
    String(candidate.value) === String(option.currentValue)
  )?.name ?? String(option.currentValue);
}

// Claude Code exposes tier aliases and a custom model as separate ACP rows,
// even when a gateway maps several rows to the same upstream model. Present
// the provider contract: one row per real model and distinct effort behavior.
export function providerConfigOptions(
  provider: string | undefined,
  options: readonly ConfigOption[],
): ConfigOption[] {
  if (provider !== "claude-deepseek" && provider !== "codex-deepseek") {
    return [...options];
  }
  return options.map((option) => {
    if (option.id === "deepseek_context") {
      return deepseekContextOption(provider, option);
    }
    if (provider === "claude-deepseek" && option.id === "model") {
      const modelFor = new Map(
        option.options.map((candidate) => [
          String(candidate.value),
          candidate.value === "default"
            ? "deepseek-v4-flash"
            : candidate.name.replace(/\[1m\]$/, ""),
        ]),
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
        currentValue: currentModel
          ? representative.get(currentModel) ?? option.currentValue
          : option.currentValue,
        options: ["deepseek-v4-flash", "deepseek-v4-pro"].flatMap((model) => {
          const value = representative.get(model);
          const candidate = option.options.find((item) => item.value === value);
          return candidate ? [candidate] : [];
        }),
      };
    }
    if (provider === "codex-deepseek" && option.id === "model") {
      const candidates = option.options.filter((candidate) =>
        String(candidate.value) !== "default"
      );
      const flash = candidates.find((candidate) =>
        /deepseek-v4-flash/iu.test(
          `${String(candidate.value)} ${candidate.name}`,
        )
      );
      return {
        ...option,
        currentValue: String(option.currentValue) === "default" && flash
          ? flash.value
          : option.currentValue,
        options: candidates.map((candidate) => ({
          ...candidate,
          name: withoutRecommendation(candidate.name),
        })),
      };
    }
    if (option.id === "effort" || option.id === "reasoning_effort") {
      const currentIsAutomatic = String(option.currentValue) === "default";
      return {
        ...option,
        name: "Reasoning effort",
        description: "DeepSeek reasoning effort for this session",
        options: option.options
          .filter((candidate) =>
            (provider !== "claude-deepseek" ||
              ["default", "high", "max"].includes(String(candidate.value))) &&
            (String(candidate.value) !== "default" || currentIsAutomatic)
          )
          .map((candidate) =>
            candidate.value === "default"
              ? { ...candidate, name: "Automatic" }
              : { ...candidate, name: withoutRecommendation(candidate.name) }
          ),
      };
    }
    return {
      ...option,
      options: option.options.map((candidate) => ({
        ...candidate,
        name: withoutRecommendation(candidate.name),
      })),
    };
  });
}
