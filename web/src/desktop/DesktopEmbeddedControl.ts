import { alpha, type Theme } from "@mui/material";

/** One geometry vocabulary for Desktop's keyboard-first surfaces. Outer
 * controls/panels share the same silhouette; nested rows and tiles use the
 * smaller inset radius. Mobile intentionally owns a separate touch language. */
export const DESKTOP_SURFACE_RADIUS = 10;
export const DESKTOP_INSET_RADIUS = 6;

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

/** Repeated Desktop rows are intentionally quieter than standalone controls.
 * Keep 1px geometry stable, but reveal the boundary only for selection,
 * keyboard focus, and hover so long lists do not become a wall of capsules. */
export function desktopListItemSx() {
  const selected = {
    borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.34),
    bgcolor: (theme: Theme) => alpha(theme.palette.primary.main, 0.065),
    boxShadow: (theme: Theme) =>
      `inset 0 0 0 1px ${alpha(theme.palette.primary.main, 0.07)}`,
  };
  return {
    border: 1,
    borderStyle: "solid",
    borderColor: "transparent",
    borderRadius: `${DESKTOP_INSET_RADIUS}px`,
    bgcolor: "transparent",
    boxShadow: "none",
    transition:
      "background-color 120ms ease, border-color 120ms ease, box-shadow 120ms ease",
    "&:hover": {
      borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.18),
      bgcolor: (theme: Theme) => alpha(theme.palette.primary.main, 0.045),
    },
    "&.Mui-selected, &[data-desktop-current='true'], &:focus-within": selected,
    "&.Mui-selected:hover": {
      borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.44),
      bgcolor: (theme: Theme) => alpha(theme.palette.primary.main, 0.09),
    },
    "&.Mui-focusVisible": {
      borderColor: "primary.main",
      boxShadow: (theme: Theme) =>
        `0 0 0 2px ${alpha(theme.palette.primary.main, 0.16)}`,
    },
  };
}
