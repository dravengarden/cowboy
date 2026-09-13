import { alpha, type Theme } from "@mui/material";

/** Far-side corners stay pill-like. The edge facing the speaker is tighter,
 * so left and right bubbles are mirrors rather than identical rounded rects. */
export const MESSAGE_BUBBLE_RADIUS_PX = 18;
export const MESSAGE_BUBBLE_EDGE_RADIUS_PX = 6;

export function messageBubbleBorderRadius(
  role: "user" | "assistant",
): string {
  const round = `${MESSAGE_BUBBLE_RADIUS_PX}px`;
  const edge = `${MESSAGE_BUBBLE_EDGE_RADIUS_PX}px`;
  return role === "user"
    ? `${round} ${edge} ${edge} ${round}`
    : `${edge} ${round} ${round} ${edge}`;
}

export function messageBubbleSurfaceSx(
  role: "user" | "assistant",
  theme: Theme,
  failed = false,
): Record<string, unknown> {
  const borderRadius = messageBubbleBorderRadius(role);
  if (role === "user") {
    return {
      border: failed ? "1px solid" : "none",
      borderColor: failed ? "error.main" : "transparent",
      borderRadius,
      bgcolor: "primary.main",
      color: "primary.contrastText",
      overflow: "hidden",
    };
  }
  return {
    border: "none",
    borderRadius,
    bgcolor: alpha(
      theme.palette.text.primary,
      theme.palette.mode === "dark" ? 0.08 : 0.05,
    ),
    color: "text.primary",
    overflow: "hidden",
  };
}
