import { alpha, type Theme } from "@mui/material";

/** Three corners stay pill-like. Only the bottom corner on the speaker side
 * is tighter — assistant bottom-left, user bottom-right. */
export const MESSAGE_BUBBLE_RADIUS_PX = 18;
export const MESSAGE_BUBBLE_TAIL_RADIUS_PX = 6;

export function messageBubbleBorderRadius(
  role: "user" | "assistant",
): string {
  const round = `${MESSAGE_BUBBLE_RADIUS_PX}px`;
  const tail = `${MESSAGE_BUBBLE_TAIL_RADIUS_PX}px`;
  return role === "user"
    ? `${round} ${round} ${tail} ${round}`
    : `${round} ${round} ${round} ${tail}`;
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
