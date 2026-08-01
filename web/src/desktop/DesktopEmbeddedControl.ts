import { alpha, type Theme } from "@mui/material";

/** One geometry vocabulary for Desktop's keyboard-first surfaces. Outer
 * controls/panels share the same silhouette; nested rows and tiles use the
 * smaller inset radius. Mobile intentionally owns a separate touch language. */
export const DESKTOP_SURFACE_RADIUS = 14;
export const DESKTOP_INSET_RADIUS = 10;

export function desktopSurfaceSx({
  active = false,
  open = false,
  interactive = true,
  focusWithin = false,
}: {
  active?: boolean;
  open?: boolean;
  interactive?: boolean;
  focusWithin?: boolean;
} = {}) {
  const focus = {
    borderColor: "primary.main",
    boxShadow: (theme: Theme) =>
      `0 0 0 3px ${alpha(theme.palette.primary.main, 0.18)}`,
  };
  return {
    border: 1,
    borderStyle: "solid",
    borderColor: (theme: Theme) =>
      alpha(theme.palette.primary.main, open ? 0.68 : active ? 0.5 : 0.3),
    borderRadius: `${DESKTOP_SURFACE_RADIUS}px`,
    bgcolor: (theme: Theme) =>
      alpha(
        theme.palette.background.paper,
        open ? (theme.palette.mode === "dark" ? 0.78 : 0.82) : 0.46,
      ),
    boxShadow: open
      ? (theme: Theme) => `0 0 0 2px ${alpha(theme.palette.primary.main, 0.1)}`
      : "none",
    transition:
      "background-color 120ms ease, border-color 120ms ease, box-shadow 120ms ease",
    ...(interactive && { "&:hover": {
      borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.52),
      bgcolor: (theme: Theme) => alpha(theme.palette.primary.main, 0.06),
    } }),
    "&.Mui-focusVisible": focus,
    ...(focusWithin && { "&:focus-within": focus }),
  };
}

/** Compact action/state control containing its visible shortcut. */
export function desktopEmbeddedControlSx(options: Parameters<typeof desktopSurfaceSx>[0] = {}) {
  return desktopSurfaceSx(options);
}
