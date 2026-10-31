// Shared geometry for the Composer's interactive cards (tool permission,
// question/elicitation, managed-call approval). Card chrome — headings,
// labels, buttons, inputs — uses the UI font; only the agent's own prose
// follows the reading font, like the Transcript it came from.

export const approvalProseSx = {
  fontFamily: "var(--cowboy-reading-font, inherit)",
} as const;

/** One decision button: a 44px touch target on Mobile, compact on fine
 *  pointers, UI type at body size. */
export const approvalButtonSx = {
  minHeight: { xs: 44, sm: 36 },
  py: 0.5,
  px: 1.5,
  borderRadius: 1.5,
  textTransform: "none",
  fontSize: "0.9375rem",
  fontWeight: 600,
  lineHeight: 1.3,
} as const;

/** Two short answers (Submit/Decline, Allow/Reject) share one row; longer or
 *  more options stack so every label stays whole. */
export function approvalChoicesInRow(labels: readonly string[]): boolean {
  return labels.length === 2 && labels.every((label) => label.length <= 18);
}
