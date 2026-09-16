// The canonical "floating decision" material — the Cancel / confirm bar at the
// bottom of every mobile sheet (session settings, New session, and every
// ConfirmSheet). ONE module so the shelf can never drift between surfaces.
//
// The look: the bar reads as a plate the form scrolls UNDER, lit by the app's
// accent — a theme-tinted hairline, a soft accent riser fading up into the
// content, and an accent glow under the confirm button.
//
// ⚠️ It is deliberately NOT a `box-shadow` on the bar. A wide upward shadow was
// tried here before (`0 -8px 18px` on a 1px strip) and iOS WebKit composited it
// over the frosted cover sheet as a LARGE PURPLE RECTANGLE — see
// `fix(mobile): flatten new session action footer` and the guard in
// MobileDecisionActions.test.ts. A gradient is ordinary paint with no
// shadow/backdrop-filter compositing interaction, so it cannot regress that way,
// and it fades softer than a shadow anyway. The confirm button keeps a REAL
// shadow because a small opaque button is the ordinary shadow case (and MUI
// already ships elevation there); the app-wide `disableElevation` default means
// this glow stays scoped to the decision shelf and nowhere else.
import { alpha, type Theme } from "@mui/material/styles";

/** How far the accent riser reaches up into the scrolling body. */
export const DECISION_RISER_PX = 22;

/** Accent glow + press physics for whatever buttons a caller puts on the shelf.
 *  Selectors are doubled so this wins over MUI's own contained/text rules
 *  regardless of stylesheet order — `actions` children come from foreign
 *  call sites (ConfirmSheet's NetworkButton), not just this module's JSX. */
export function decisionActionEmphasis(t: Theme): Record<string, unknown> {
  const dark = t.palette.mode === "dark";
  const accent = t.palette.primary.main;
  return {
    "& .MuiButton-contained.MuiButton-contained": {
      // Tight geometry on purpose: the glow hugs the button instead of washing
      // the strip, which is what makes it read as LIFTED rather than blurry.
      boxShadow: `0 6px 16px -8px ${
        alpha(accent, dark ? 0.9 : 0.55)
      }, 0 2px 6px -4px ${alpha(accent, dark ? 0.7 : 0.4)}`,
      transition: t.transitions.create(["box-shadow", "transform"], {
        duration: t.transitions.duration.shortest,
      }),
      "&:active": {
        // Press = settle onto the surface. The pair (shadow collapses AND the
        // cap moves down 1px) is what sells contact; either alone reads as a
        // colour change.
        boxShadow: `0 1px 4px -3px ${alpha(accent, dark ? 0.8 : 0.5)}`,
        transform: "translateY(1px)",
      },
      // An unavailable action must not advertise depth it cannot deliver.
      "&.Mui-disabled": { boxShadow: "none", transform: "none" },
      "@media (prefers-reduced-motion: reduce)": { transition: "none" },
    },
    // The quiet half of the pair: a tonal pill, so Cancel is still one of two
    // controls rather than stray text next to a button.
    "& .MuiButton-text.MuiButton-text": {
      backgroundColor: alpha(t.palette.text.primary, dark ? 0.08 : 0.05),
      "&:active": {
        backgroundColor: alpha(t.palette.text.primary, dark ? 0.14 : 0.09),
      },
      "&.Mui-disabled": { backgroundColor: "transparent" },
    },
  };
}

/** The plate itself. Spread onto the element that already bleeds to the sheet's
 *  edges; it adds the surface, the accent hairline, and the riser. */
export function decisionShelfSurface(t: Theme): Record<string, unknown> {
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
    "&::before": {
      content: '""',
      position: "absolute",
      left: 0,
      right: 0,
      bottom: "100%",
      height: DECISION_RISER_PX,
      pointerEvents: "none",
      // Two stacked fades: a neutral one for depth against any content, and the
      // accent above it for the hue. Both end in a ZERO-ALPHA version of their
      // own colour — `transparent` fades through black in Safari.
      backgroundImage: [
        `linear-gradient(to top, ${alpha(accent, dark ? 0.2 : 0.12)}, ${
          alpha(accent, 0)
        })`,
        `linear-gradient(to top, ${alpha(shade, dark ? 0.32 : 0.09)}, ${
          alpha(shade, 0)
        })`,
      ].join(", "),
    },
    ...decisionActionEmphasis(t),
  };
}
