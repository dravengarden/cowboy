import { alpha, type Theme } from "@mui/material/styles";

/** Bottom edge of every floating control island (Sessions, sheets, Review):
 *  on the home-indicator inset, or 12px above the edge without one. */
export const FLOATING_CONTROLS_BOTTOM = "max(env(safe-area-inset-bottom, 0px), 12px)";

/** One bounded progressive frost scrim behind floating controls, never on
 *  scrolling content. Pass `surfaceColor` when hosted on `background.paper`
 *  (e.g. compact bottom sheets) so content melts seamlessly into that surface. */
export function readerBottomEdge(theme: Theme, surfaceColor?: string) {
  const color = surfaceColor ?? theme.palette.background.default;
  const mask = "linear-gradient(to bottom, transparent, #000 56px)";
  return {
    isolation: "isolate",
    "&::before": {
      content: '""',
      position: "absolute",
      inset: "-40px 0 0",
      zIndex: -1,
      pointerEvents: "none",
      backdropFilter: "blur(8px)",
      WebkitBackdropFilter: "blur(8px)",
      maskImage: mask,
      WebkitMaskImage: mask,
      background: `linear-gradient(to bottom, ${alpha(color, 0)} 0%, ${alpha(color, 0.56)} 34px, ${alpha(color, 0.92)} 64px, ${color} 100%)`,
      "[data-detent-moving] &": {
        backdropFilter: "none",
        WebkitBackdropFilter: "none",
        background: `linear-gradient(to bottom, ${alpha(color, 0)} 0%, ${color} 56px)`,
      },
      "@media (prefers-reduced-transparency: reduce)": {
        backdropFilter: "none",
        WebkitBackdropFilter: "none",
        background: color,
      },
    },
  };
}

/** Unified Cowboy floating island material: calm plum-tinted frosted matte
 *  glass with a crisp hairline rim and bounded single-pass blur (zero fake
 *  radial spotlight glare, zero stacked pseudo-element blur passes, and
 *  automatic opaque fallback during sheet translation and reduced-transparency
 *  mode for 60-120fps iOS smoothness). */
export function floatingIslandSurfaceSx(
  theme: Theme,
  options: { readonly softRim?: boolean; readonly compact?: boolean } = {},
): Record<string, unknown> {
  const dark = theme.palette.mode === "dark";
  const softRim = options.softRim === true;
  const compact = options.compact === true;
  return {
    border: softRim ? 0 : 1,
    borderColor: dark
      ? alpha(theme.palette.common.white, softRim ? 0.08 : 0.13)
      : alpha(theme.palette.common.black, softRim ? 0.06 : 0.10),
    bgcolor: alpha(theme.palette.background.paper, dark ? 0.82 : 0.86),
    backgroundImage: dark
      ? `linear-gradient(180deg, ${alpha(theme.palette.common.white, 0.055)} 0%, ${alpha(theme.palette.primary.main, 0.065)} 100%)`
      : `linear-gradient(180deg, ${alpha(theme.palette.common.white, 0.6)} 0%, ${alpha(theme.palette.primary.main, 0.04)} 100%)`,
    boxShadow: compact
      ? `0 4px 14px ${alpha(theme.palette.common.black, dark ? 0.24 : 0.08)}`
      : [
          `0 6px 20px ${alpha(theme.palette.common.black, dark ? 0.36 : 0.10)}`,
          `inset 0 1px 0 ${alpha(theme.palette.common.white, dark ? 0.09 : 0.82)}`,
        ].join(", "),
    backdropFilter: "blur(14px) saturate(150%)",
    WebkitBackdropFilter: "blur(14px) saturate(150%)",
    "[data-detent-moving] &": {
      backdropFilter: "none",
      WebkitBackdropFilter: "none",
      bgcolor: theme.palette.background.paper,
    },
    "@media (prefers-reduced-transparency: reduce)": {
      backdropFilter: "none",
      WebkitBackdropFilter: "none",
      bgcolor: theme.palette.background.paper,
    },
  };
}
