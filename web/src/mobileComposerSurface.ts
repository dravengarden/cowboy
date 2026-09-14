import { alpha, type Theme } from "@mui/material";
import { mobileComposerPanelFrameSx } from "./mobileComposerPrimitives";

/** The focused Mobile writing material shared by the primary input and
 * Queue/Draft row editors. Keeping it shared prevents pending edits from
 * falling back to a transparent panel that lets transcript text show through.
 * The fill is fully opaque: iOS composites CodeMirror onto its own layer, so
 * any alpha still samples the transcript and reads as a hole. */
export function mobileFocusedComposerFill(theme: Theme): string {
  return theme.palette.background.paper;
}

/** Outer card stroke. Strong enough to read as the writing instrument's
 * silhouette, quiet enough not to compete with primary actions. */
export function mobileComposerOutlineColor(theme: Theme): string {
  return alpha(
    theme.palette.primary.main,
    theme.palette.mode === "dark" ? 0.5 : 0.42,
  );
}

/** Inner rails (track split + trailing-action column). Same hue as the
 * outline, at a lower weight so the two-track dock still reads as one card
 * rather than a grid of gray scaffolding. */
export function mobileComposerHairlineColor(theme: Theme): string {
  return alpha(
    theme.palette.primary.main,
    theme.palette.mode === "dark" ? 0.36 : 0.28,
  );
}

/** Tight themed halo on the OUTER card only: a 1px optical ring, a short
 * bloom, then the existing lift. Inner descendants stay paint-only
 * (`border-color`) so the peek compositor does not grow extra shadow tiles. */
export function mobileComposerOutlineGlow(theme: Theme): string {
  const primary = theme.palette.primary.main;
  const dark = theme.palette.mode === "dark";
  return [
    `0 0 0 1px ${alpha(primary, dark ? 0.18 : 0.08)}`,
    `0 0 ${dark ? 14 : 10}px ${alpha(primary, dark ? 0.26 : 0.14)}`,
    `0 10px 28px ${alpha(theme.palette.common.black, dark ? 0.24 : 0.09)}`,
  ].join(", ");
}

export const mobileFocusedComposerSurfaceSx = {
  borderColor: mobileComposerOutlineColor,
  borderRadius: mobileComposerPanelFrameSx.borderRadius,
  bgcolor: mobileFocusedComposerFill,
  overflow: "hidden",
  boxShadow: mobileComposerOutlineGlow,
} as const;
