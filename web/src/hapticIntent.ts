export type CowboyHapticIntent =
  | "navigation"
  | "magnetic"
  | "confirmation"
  | "important";

export type CowboyHapticStyle = "light" | "medium" | "heavy";

export function hapticStyleForIntent(
  intent: CowboyHapticIntent,
): CowboyHapticStyle {
  if (intent === "navigation") return "light";
  if (intent === "magnetic" || intent === "confirmation") return "medium";
  return "heavy";
}
