// Shared "liquid glass" frosted surfaces — the SINGLE source for the floating
// overlays' material (Transcript/Composer turn-status pills + the permission
// overlay). iOS-style
// vibrancy: a heavy blur+saturate+brightness for the lens, a soft drop shadow to
// float it, and (for the pill) a bottom inner shadow for glass thickness + a flat
// tone tint. Legibility comes from the blur, not opacity.
//
// Returned as plain style objects so a caller spreads them into ONE `sx={(t) =>
// ({ ...frostedPill(t, tone), ...local })}` theme callback.
import { alpha, type Theme } from "@mui/material/styles";
import { FROSTED_PILL_DROP_SHADOW_GEOMETRY } from "./floatingOverlayPolicy";

const BLUR = "blur(40px) saturate(180%) brightness(1.06)";

/** Resting Mobile chrome (composer + navbar). One blur tile at rest;
 *  drawer flatten strips the filter at prepare, before the first
 *  translate, so the swipe does not relocate a backdrop-filter layer. */
export function frostedChrome(t: Theme): Record<string, unknown> {
  const dark = t.palette.mode === "dark";
  return {
    backgroundColor: alpha(t.palette.background.default, dark ? 0.58 : 0.64),
    backdropFilter: BLUR,
    WebkitBackdropFilter: BLUR,
  };
}

// The denser panel surface (no tone tint) — for a block of raw text / controls.
export function frostedPanel(t: Theme): Record<string, unknown> {
  const dark = t.palette.mode === "dark";
  return {
    backgroundColor: alpha(t.palette.background.default, dark ? 0.46 : 0.54),
    backdropFilter: BLUR,
    WebkitBackdropFilter: BLUR,
    boxShadow: `0 8px 28px ${alpha(t.palette.common.black, dark ? 0.5 : 0.18)}`,
  };
}

// The pill surface — a lighter base + a flat `toneMain` tint (the colour-coded
// state) + a thickness inner shadow.
export function frostedPill(t: Theme, toneMain: string): Record<string, unknown> {
  const dark = t.palette.mode === "dark";
  const tint = alpha(toneMain, dark ? 0.16 : 0.2);
  return {
    backgroundColor: alpha(t.palette.background.default, dark ? 0.34 : 0.4),
    backgroundImage: `linear-gradient(0deg, ${tint}, ${tint})`,
    backdropFilter: BLUR,
    WebkitBackdropFilter: BLUR,
    boxShadow: [
      // Keep the elevation close to the pill. A broad downward shadow used to
      // cross the boundary seam and visually merge Judging with the Composer.
      `${FROSTED_PILL_DROP_SHADOW_GEOMETRY} ${
        alpha(t.palette.common.black, dark ? 0.5 : 0.18)
      }`,
      `inset 0 -9px 14px -8px ${alpha(t.palette.common.black, dark ? 0.4 : 0.12)}`,
    ].join(", "),
  };
}
