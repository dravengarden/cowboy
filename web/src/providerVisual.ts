export type ThemeMode = "light" | "dark";

export interface ProviderVisual {
  primary: string;
  secondary: string;
}

// Runtime variants combine two identities: DeepSeek owns the primary blue,
// while the agent family owns the secondary hue. This keeps both DeepSeek
// lanes coherent without making Claude and Codex indistinguishable.
export function providerVisual(
  provider: string,
  mode: ThemeMode,
): ProviderVisual {
  const dark = mode === "dark";
  switch (provider) {
    case "claude-code":
      return { primary: "#D97757", secondary: dark ? "#EDA78E" : "#B95F43" };
    case "claude-deepseek":
      return {
        primary: dark ? "#8EA2FF" : "#4D6BFE",
        secondary: dark ? "#C7A7FF" : "#805AD5",
      };
    case "codex-deepseek":
      return {
        primary: dark ? "#8EA2FF" : "#4D6BFE",
        secondary: dark ? "#62D6BC" : "#168B78",
      };
    case "reasonix-deepseek":
      return {
        primary: dark ? "#8EA2FF" : "#4D6BFE",
        secondary: dark ? "#55D6FF" : "#1687B8",
      };
    case "codex":
      return {
        primary: dark ? "#8FA8FF" : "#4F6BED",
        secondary: dark ? "#62D6BC" : "#168B78",
      };
    default:
      return {
        primary: dark ? "#A9B4C7" : "#52606D",
        secondary: dark ? "#D1D8E5" : "#7B8794",
      };
  }
}
