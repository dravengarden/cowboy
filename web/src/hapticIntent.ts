export type CowboyHapticIntent =
  | "navigation"
  | "magnetic"
  | "confirmation"
  | "important";

export type CowboyHapticStyle = "selection" | "light" | "heavy";

export function hapticStyleForIntent(
  intent: CowboyHapticIntent,
): CowboyHapticStyle {
  // Strength follows consequence, not the widget.
  // Browse / spatial cues stay on UISelectionFeedbackGenerator.
  // Ordinary commits use a light impact. Only destructive confirmations
  // escalate to heavy.
  if (intent === "navigation" || intent === "magnetic") return "selection";
  if (intent === "confirmation") return "light";
  return "heavy";
}
