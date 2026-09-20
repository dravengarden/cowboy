import { alpha, type Theme } from "@mui/material";

/** One geometry vocabulary for Desktop's keyboard-first surfaces. Outer
 * controls, panels, and first-level interactive rows share the same silhouette;
 * only content nested inside them uses the smaller inset radius. Mobile
 * intentionally owns a separate touch language. */
export const DESKTOP_SURFACE_RADIUS = 10;
export const DESKTOP_INSET_RADIUS = 6;
/** Shared top-bar control geometry. Keep every first-level control on the
 * same baseline, including the nested session lifecycle cluster. */
export const DESKTOP_TOPBAR_CONTROL_HEIGHT = 38;
export const DESKTOP_TOPBAR_CONTROL_GAP = 0.75;
export const DESKTOP_TOPBAR_CONTROL_GAP_PX = 6;

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
    // An idle control is geometry, not selection. Keeping a primary-tinted
    // border here made every control in a focused toolbar look active at once.
    // Primary belongs only to an explicitly active/open control; keyboard
    // focus is painted independently by `.Mui-focusVisible` below.
    borderColor: (theme: Theme) =>
      open
        ? alpha(theme.palette.primary.main, 0.68)
        : active
        ? alpha(theme.palette.primary.main, 0.5)
        : theme.palette.divider,
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

/** Shared geometry for the compact session-lifecycle cluster in Desktop's top
 * bar. Keep every action mounted at the same height so state changes only
 * affect availability, never the toolbar's silhouette. */
export function desktopSessionActionSx({
  active = false,
  open = false,
  minWidth = 80,
}: {
  active?: boolean;
  open?: boolean;
  minWidth?: number;
} = {}) {
  return {
    ...desktopEmbeddedControlSx({ active, open }),
    height: DESKTOP_TOPBAR_CONTROL_HEIGHT,
    minWidth,
    px: 0.75,
    flexShrink: 0,
    textTransform: "none",
    whiteSpace: "nowrap",
    "& .MuiButton-startIcon": { mr: 0.5 },
  };
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
    borderRadius: `${DESKTOP_SURFACE_RADIUS}px`,
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

/** The house Desktop modal material. Dark mode's `background.paper` is very
 * close to the canvas, so a dialog that keeps MUI's flat default paper reads
 * as an undelimited black slab with no visible edge. Every Desktop dialog —
 * `DesktopModal` and the `Sheet`'s Desktop branch — shares this one frosted,
 * edged, shadowed material so no surface can drift back to the default. */
export function desktopModalPaperSx() {
  return {
    overflow: "hidden",
    borderRadius: `${DESKTOP_SURFACE_RADIUS}px`,
    border: 1,
    borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.22),
    bgcolor: (theme: Theme) => alpha(theme.palette.background.paper, 0.96),
    backgroundImage: (theme: Theme) =>
      `linear-gradient(145deg, ${
        alpha(
          theme.palette.common.white,
          theme.palette.mode === "dark" ? 0.035 : 0.42,
        )
      }, transparent 48%)`,
    backdropFilter: "blur(28px) saturate(145%)",
    boxShadow: (theme: Theme) =>
      `0 28px 80px ${
        alpha(
          theme.palette.common.black,
          theme.palette.mode === "dark" ? 0.48 : 0.22,
        )
      }`,
  };
}

/** Scrim behind a Desktop modal. Paired with `desktopModalPaperSx` so the
 * dialog's own edge always has something to separate from. */
export function desktopModalBackdropSx() {
  return {
    bgcolor: (theme: Theme) =>
      alpha(
        theme.palette.common.black,
        theme.palette.mode === "dark" ? 0.56 : 0.34,
      ),
    backdropFilter: "blur(3px)",
  };
}

/** A labelled group inside a Desktop modal. An outline-only group vanishes
 * into dark mode's near-black paper, which is what made the control center
 * read as one flat sheet of rows; a faint raised fill gives every section the
 * same findable edge. */
export function desktopPanelSx() {
  return {
    border: 1,
    borderColor: "divider",
    borderRadius: `${DESKTOP_SURFACE_RADIUS}px`,
    bgcolor: (theme: Theme) =>
      alpha(
        theme.palette.mode === "dark"
          ? theme.palette.common.white
          : theme.palette.common.black,
        theme.palette.mode === "dark" ? 0.035 : 0.018,
      ),
  };
}
