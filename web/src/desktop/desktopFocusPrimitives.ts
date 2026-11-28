// Single source of truth for Desktop keyboard-focus visuals across Cowboy.
//
// Every Desktop workspace region — Sessions, collapsed Sessions rail, Prompt
// Composer / Draft, Plan, Queue, Drafts, Calls, Conversation, Explore Page
// Index, Review Navigator / Document / Inspector, Tasks Roster / Reader, and
// Top Bar — shares ONE paint-only perimeter frame primitive (`::after` overlay
// with `inset: 0; pointer-events: none`).
//
// Invariants:
//   1. Zero layout shift: focus transitions never toggle border-width,
//      padding, margin, or box dimensions.
//   2. Universal coverage: zero `:not(...)` exclusions for workspace regions.
//   3. Scroll stability: every `[data-desktop-region]` is a non-scrolling
//      frame host so its `::after` perimeter stays pinned to the region
//      viewport while inner lists, transcripts, or editors scroll.
//   4. Layered hierarchy: outer 1.5px perimeter frame (region) + calm pane
//      header indicator (pane) + inner tonal/bar cursor (focused item).

import { alpha, type Theme } from "@mui/material";
import { desktopFocusFill } from "../theme";

/** Opaque primary-mixed accent edge so the perimeter reads with identical
 *  contrast over white canvases, shaded headers, companion rails, and dark
 *  editor backgrounds. */
export function desktopFocusEdgeColor(theme: Theme): string {
  const weight = theme.palette.mode === "dark" ? 68 : 56;
  return `color-mix(in srgb, ${theme.palette.primary.main} ${String(weight)}%, ${theme.palette.background.default})`;
}

/** Soft inner halo paired with the crisp 1.5px perimeter edge so peripheral
 *  vision locates the active region immediately on wide displays. */
export function desktopFocusHaloColor(theme: Theme): string {
  return alpha(
    theme.palette.primary.main,
    theme.palette.mode === "dark" ? 0.14 : 0.085,
  );
}

/** Paint-only inset perimeter frame for the single focused Desktop region. */
export function desktopRegionFocusBoxShadow(theme: Theme): string {
  return `inset 0 0 0 1.5px ${desktopFocusEdgeColor(theme)}, inset 0 0 0 4px ${desktopFocusHaloColor(theme)}`;
}

/** Subtle surface lift for bounded auxiliary cards (Plan, Queue, Drafts,
 *  Calls) while they own region focus. Writing/reading canvases stay pure. */
export function desktopAuxRegionFocusBg(theme: Theme): string {
  return alpha(
    theme.palette.primary.main,
    theme.palette.mode === "dark" ? 0.055 : 0.03,
  );
}

/** Shared control / card focus ring aligned with the region perimeter palette. */
export function desktopControlFocusRing(theme: Theme): string {
  return `0 0 0 2px ${desktopFocusHaloColor(theme)}`;
}

/** Root Desktop focus stylesheet mounted on the main Session workspace
 *  (`App.tsx`), standalone Draft workspace (`DraftWorkspace.tsx`), and
 *  portaled modal workspaces (`DesktopModal.tsx`). */
export function desktopFocusRootSx(currentRowFill = 0.07): Record<string, unknown> {
  return {
    // Tier 1: Universal paint-only perimeter frame on EVERY Desktop region.
    // Zero perimeter exclusions — every region paints the exact same `::after`
    // overlay; inset overlays keep `position: absolute` via host attribute.
    "& [data-desktop-region]": {
      position: "relative",
      outline: "none",
      transition: "background-color 120ms ease",
      "&[data-desktop-region-position='absolute']": {
        position: "absolute",
      },
      "&::after": {
        content: '""',
        position: "absolute",
        inset: 0,
        borderRadius: "inherit",
        pointerEvents: "none",
        zIndex: 3,
        boxShadow: "none",
        transition: "box-shadow 140ms ease",
      },
    },
    "& [data-desktop-region][data-desktop-focused='true']::after": {
      boxShadow: desktopRegionFocusBoxShadow,
      "@media (forced-colors: active)": {
        boxShadow: "none",
        border: "2px solid Highlight",
      },
    },
    // Bounded Prompt auxiliary docks add a calm surface lift beneath the
    // shared perimeter frame so they separate softly inside stacked/companion
    // columns without altering layout.
    "& [data-desktop-region='prompt.queued'][data-desktop-focused='true'], & [data-desktop-region='prompt.draft'][data-desktop-focused='true']": {
      bgcolor: desktopAuxRegionFocusBg,
    },
    "& [data-desktop-region='prompt.calls'][data-desktop-focused='true'] > [data-desktop-calls-surface], & [data-desktop-region='prompt.plan'][data-desktop-focused='true'] > [data-desktop-plan-surface]": {
      bgcolor: desktopAuxRegionFocusBg,
    },
    // The full-height Composer delegates its perimeter focus cue to the shared
    // region `::after` overlay; suppress native browser focus rings.
    "& [data-desktop-region='prompt.composer']:focus-visible": {
      outline: "none",
      boxShadow: "none",
    },
    // Tier 2: Calm, symmetrical pane-header grounding across Sessions, Prompt,
    // and Conversation — text elevation + shared 6px dot, zero harsh top bars.
    "& [data-desktop-pane] > [data-desktop-pane-header]": {
      color: "text.secondary",
      bgcolor: (t: Theme) =>
        alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.14 : 0.022),
      boxShadow: "none",
      transition: "background-color 140ms ease, color 140ms ease",
    },
    "& [data-desktop-pane][data-desktop-pane-focused='true'] > [data-desktop-pane-header]": {
      color: "text.primary",
      bgcolor: "background.default",
      boxShadow: "none",
    },
    "& [data-desktop-pane][data-desktop-pane-focused='true'] > [data-desktop-pane-header] [data-desktop-pane-dot]": {
      opacity: 1,
    },
    // Top Bar combines the shared perimeter frame with a quiet fill while
    // keeping `color="inherit"` children neutral until individually focused.
    "& [data-desktop-region='topbar.controls'][data-desktop-focused='true']": {
      bgcolor: desktopFocusFill,
      boxShadow: "none",
    },
    "& [data-desktop-region='topbar.controls'][data-desktop-focused='true'] [data-desktop-item]:focus": {
      outline: "none",
      borderColor: (t: Theme) => desktopFocusEdgeColor(t),
      bgcolor: (t: Theme) => alpha(t.palette.primary.main, t.palette.mode === "dark" ? 0.14 : 0.1),
      boxShadow: "none",
    },
    // Tier 3: Unified item-level keyboard cursor inside focused regions.
    "& [data-desktop-item]:focus-visible": {
      outline: "none",
      bgcolor: (t: Theme) => alpha(t.palette.primary.main, t.palette.mode === "dark" ? 0.12 : 0.09),
      boxShadow: (t: Theme) => `inset 2px 0 0 ${t.palette.primary.main}`,
      "@media (forced-colors: active)": {
        outline: "2px solid Highlight",
        outlineOffset: "-2px",
      },
    },
    // Session selection and keyboard focus stay distinct: the filled row is
    // the open session, while j/k moves a keyboard cursor that l/Enter opens.
    // MUI's :focus-visible is not retained reliably across programmatic
    // focus() on macOS, so draw the cursor from real DOM :focus while Sessions
    // owns focus.
    "& [data-desktop-region='sessions.list'][data-desktop-focused='true'] [data-desktop-item]:focus": {
      outline: "none",
      bgcolor: (t: Theme) => alpha(t.palette.primary.main, t.palette.mode === "dark" ? 0.08 : 0.05),
      boxShadow: (t: Theme) => `inset 2px 0 0 ${t.palette.primary.main}`,
      "@media (forced-colors: active)": {
        outline: "2px solid Highlight",
        outlineOffset: "-2px",
      },
      "& .cowboy-session-grip": {
        color: "primary.main",
      },
      "& .cowboy-session-shortcut": {
        opacity: "1 !important",
      },
    },
    "& [data-desktop-region='sessions.list'][data-desktop-focused='true'] [data-desktop-item][data-desktop-current='true']:focus": {
      bgcolor: (t: Theme) => alpha(t.palette.primary.main, currentRowFill + 0.06),
      boxShadow: "none",
    },
    "& [data-desktop-region='sessions.list'][data-desktop-pinned='true'] [data-desktop-item][data-desktop-pin-active='true']:focus": {
      bgcolor: (t: Theme) => alpha(t.palette.primary.main, 0.105),
      boxShadow: (t: Theme) =>
        `inset 0 0 0 2px ${desktopFocusEdgeColor(t)}, 0 3px 14px ${alpha(t.palette.primary.main, 0.09)}`,
    },
  };
}
