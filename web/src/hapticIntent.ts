export type CowboyHapticIntent =
  | "navigation"
  | "magnetic"
  | "confirmation"
  | "important";

export type CowboyHapticStyle = "selection" | "medium" | "heavy";

export function hapticStyleForIntent(
  intent: CowboyHapticIntent,
): CowboyHapticStyle {
  // Spatial/navigation thresholds are quiet orientation cues, not impacts.
  // UISelectionFeedbackGenerator is substantially subtler than even a "light"
  // UIImpactFeedbackGenerator and matches native drawer/magnetic affordances.
  if (intent === "navigation" || intent === "magnetic") return "selection";
  if (intent === "confirmation") return "medium";
  return "heavy";
}
