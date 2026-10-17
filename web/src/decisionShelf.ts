// The canonical "floating decision" material — the Cancel / confirm bar at the
// bottom of every mobile sheet (session settings, New session, and every
// ConfirmSheet). ONE module so the shelf can never drift between surfaces.
//
// The look: two equal columns on a plate the form scrolls UNDER, lit by the
// colour of the decision itself — a theme-tinted hairline, an accent riser
// fading up into the content, and quiet, stationary press feedback shared by both choices.
//
// ⚠️ The plate is deliberately NOT a `box-shadow`. A wide upward shadow was
// tried here before (`0 -8px 18px` on a 1px strip) and iOS WebKit composited it
// over the frosted cover sheet as a LARGE PURPLE RECTANGLE — see
// `fix(mobile): flatten new session action footer` and the guard in
// MobileDecisionActions.test.ts. A gradient is ordinary paint with no
// shadow/backdrop-filter compositing interaction, so it cannot regress that way,
// and it fades softer than a shadow anyway. Button feedback is paint-only.
import { alpha, type Theme } from "@mui/material/styles";

/** How far the accent riser reaches up into the scrolling body. */
export const DECISION_RISER_PX = 22;

/** Quiet, shadow-free form variant; press feedback comes from decisionActionEmphasis.
 *  Selectors are doubled so this wins over MUI's own contained/text rules
 *  regardless of stylesheet order — `actions` children come from foreign
 *  call sites (ConfirmSheet's NetworkButton), not just this module's JSX. */
export function flatDecisionActions(t: Theme): Record<string, unknown> {
  return {
    "& > .MuiButton-root": { flex: 1, minHeight: 44 },
    "& .MuiButton-contained.MuiButton-contained": {
      boxShadow: "none",
      "&:hover, &:active, &:focus-visible, &.Mui-disabled": {
        boxShadow: "none",
        transform: "none",
      },
    },
    "& .MuiButton-text.MuiButton-text": {
      backgroundColor: alpha(t.palette.text.primary, 0.05),
    },
  };
}

export function flatDecisionShelf(t: Theme): Record<string, unknown> {
  return {
    backgroundColor: t.palette.background.default,
    borderTop: `1px solid ${t.palette.divider}`,
    borderRadius: 0,
    boxShadow: "none",
    ...flatDecisionActions(t),
  };
}

export function decisionActionEmphasis(t: Theme): Record<string, unknown> {
  const dark = t.palette.mode === "dark";
  return {
    "& > .MuiButton-root": { flex: 1, minWidth: 0, minHeight: 44 },
    // Stable targets: feedback changes paint, never position, scale or shadow.
    // Both choices use the same 120ms state layer, including the flat variant.
    "& .MuiButton-root.MuiButton-root": {
      position: "relative",
      boxShadow: "none",
      transform: "none",
      overflowWrap: "anywhere",
      transition: "background-color 120ms ease-out, color 120ms ease-out",
      // The shared state layer replaces ripple on both pointer models.
      "& .MuiTouchRipple-root": { display: "none" },
      "&:hover, &:active, &.Mui-disabled": { boxShadow: "none", transform: "none" },
      "&::after": {
        content: '""',
        position: "absolute",
        inset: 0,
        borderRadius: "inherit",
        pointerEvents: "none",
        backgroundColor: "currentColor",
        opacity: 0,
        transition: "opacity 120ms ease-out",
      },
      "&:active:not(.Mui-disabled)::after, &[data-decision-pressed]:not(.Mui-disabled)::after": { opacity: 0.12, transitionDuration: "60ms" },
      "&.Mui-focusVisible": { outline: `2px solid ${t.palette.primary.main}`, outlineOffset: 2 },
      "@media (prefers-reduced-motion: reduce)": {
        transition: "none",
        "&::after": { transition: "none" },
      },
    },
    "& .MuiButton-text.MuiButton-text": {
      backgroundColor: alpha(t.palette.text.primary, dark ? 0.08 : 0.05),
      "&.Mui-disabled": { backgroundColor: "transparent" },
    },
  };
}

/** The plate itself. Spread onto the element that already bleeds to the sheet's
 *  edges; it adds the surface, the accent hairline, and — only when the body it
 *  sits on actually scrolls — the riser.
 *
 *  Pass `riser: false` for a short card whose content ends above the bar: the
 *  gradient means "this continues underneath", and painting it over a finished
 *  paragraph is just a smudge. */
export function decisionShelfSurface(
  t: Theme,
  { riser = true }: { riser?: boolean } = {},
): Record<string, unknown> {
  const dark = t.palette.mode === "dark";
  const accent = t.palette.primary.main;
  const shade = t.palette.common.black;
  // Flat tone tint over the surface colour (the frostedPill technique): on a
  // sheet the plate already differs from the body, but a ConfirmSheet card is
  // `background.paper` end to end, so this whisper of accent is what tells the
  // decision row from the text above it. Keep it under 6% — more reads dirty,
  // especially in light mode.
  const tint = alpha(accent, dark ? 0.06 : 0.035);
  return {
    position: "relative",
    // Opaque, and one step off the sheet body — in dark `paper` is the lighter
    // surface, in light it is the whiter one. Either way the plate separates
    // from the form without a border cage.
    backgroundColor: t.palette.background.paper,
    backgroundImage: `linear-gradient(0deg, ${tint}, ${tint})`,
    borderTopLeftRadius: 16,
    borderTopRightRadius: 16,
    // The accent, stated once and crisply. A tinted hairline carries the brand
    // at 1px where a wash would just look dirty.
    borderTop: `1px solid ${alpha(accent, dark ? 0.3 : 0.2)}`,
    ...(riser
      ? {
        "&::before": {
          content: '""',
          position: "absolute",
          left: 0,
          right: 0,
          bottom: "100%",
          height: DECISION_RISER_PX,
          pointerEvents: "none",
          // Two stacked fades: a neutral one for depth against any content, and
          // the accent above it for the hue. Both end in a ZERO-ALPHA version
          // of their own colour — `transparent` fades through black in Safari.
          backgroundImage: [
            `linear-gradient(to top, ${alpha(accent, dark ? 0.2 : 0.12)}, ${
              alpha(accent, 0)
            })`,
            `linear-gradient(to top, ${alpha(shade, dark ? 0.32 : 0.09)}, ${
              alpha(shade, 0)
            })`,
          ].join(", "),
        },
      }
      : {}),
    ...decisionActionEmphasis(t),
  };
}
