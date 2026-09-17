import { alpha, type Theme } from "@mui/material";

/** Three corners stay pill-like. Only the bottom corner on the speaker side
 * is tighter — assistant bottom-left, user bottom-right. */
export const MESSAGE_BUBBLE_RADIUS_PX = 18;
export const MESSAGE_BUBBLE_TAIL_RADIUS_PX = 6;

/** Widest image preview or file card inside a message. A pixel value, never a
 * percentage: the user bubble is `width: fit-content`, and a percentage
 * max-width is ignored while that width is computed, so a large screenshot
 * would size the bubble by its natural width and fill the whole row. */
export const MESSAGE_PREVIEW_MAX_WIDTH_PX = 360;

export function messageBubbleBorderRadius(
  role: "user" | "assistant",
): string {
  const round = `${MESSAGE_BUBBLE_RADIUS_PX}px`;
  const tail = `${MESSAGE_BUBBLE_TAIL_RADIUS_PX}px`;
  return role === "user"
    ? `${round} ${round} ${tail} ${round}`
    : `${round} ${round} ${round} ${tail}`;
}

/** Assistant replies are reading surface: use the full column. User sends
 * shrink to their text so "hi" stays a compact chip, then grow up to the
 * same column so a long prompt is not a floating island. */
export function messageBubbleLayoutSx(role: "user" | "assistant"): {
  alignSelf: "stretch" | "flex-end";
  width: string;
  maxWidth: string;
  minWidth: number;
  boxSizing: "border-box";
} {
  if (role === "user") {
    return {
      alignSelf: "flex-end",
      width: "fit-content",
      maxWidth: "100%",
      minWidth: 0,
      boxSizing: "border-box",
    };
  }
  return {
    alignSelf: "stretch",
    width: "100%",
    maxWidth: "100%",
    minWidth: 0,
    boxSizing: "border-box",
  };
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
