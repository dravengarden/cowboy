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
//   4. Layered hierarchy: hardware-snapped 1px perimeter frame + whisper-soft
//      feathered inner bloom (region) + calm pane header indicator (pane) +
//      inner tonal/bar cursor (focused item).

import { alpha, type Theme } from "@mui/material";
import { desktopFocusBoundary, desktopFocusFill } from "../theme";

/** Opaque restrained primary-mixed accent edge so the perimeter stays calm and
 *  understated while remaining immediately discernible over white canvases,
 *  shaded headers, companion rails, and dark editor backgrounds. */
export function desktopFocusEdgeColor(theme: Theme): string {
  return desktopFocusBoundary(theme);
}

/** Whisper-soft inner bloom tint paired with the pixel-snapped 1px perimeter
 *  border so active focus reads calmly without a harsh solid band. */
export function desktopFocusHaloColor(theme: Theme): string {
  return alpha(
    theme.palette.primary.main,
    theme.palette.mode === "dark" ? 0.11 : 0.065,
  );
}

/** Paint-only feathered inset glow paired with the 1px ::after border. */
export function desktopRegionFocusBoxShadow(theme: Theme): string {
  return `inset 0 0 6px 0 ${desktopFocusHaloColor(theme)}`;
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

// Option A single-region pane perimeter promotion:
// When a pane with a PaneHeader (Prompt, Conversation) or rail header
// (collapsed Sessions rail) owns focus AND contains only ONE visible region
// (not `[hidden]`, not inside `[hidden]`, and not `:empty`), promote the
// shared paint-only `::after` frame onto the outer `[data-desktop-pane]` so
// the header sits seamlessly inside the highlight with zero double seam below
// it. Multi-region panes (Review split, Tasks split, Prompt with Plan/Queue/
// Drafts) keep the perimeter tightly on the focused sub-region.
const VISIBLE_FOCUSED_REGION =
  "[data-desktop-region][data-desktop-focused='true']:not([hidden]):not([hidden] *):not(:empty)";
const VISIBLE_UNFOCUSED_REGION =
  "[data-desktop-region]:not([data-desktop-focused='true']):not([hidden]):not([hidden] *):not(:empty)";
export const PROMOTABLE_DESKTOP_PANE_SELECTOR =
  "& [data-desktop-pane]:not([data-desktop-region]):has(> [data-desktop-pane-header], > [data-desktop-sessions-rail-head])";
export const PROMOTED_DESKTOP_PANE_SELECTOR =
  `${PROMOTABLE_DESKTOP_PANE_SELECTOR}[data-desktop-pane-focused='true']:has(${VISIBLE_FOCUSED_REGION}):not(:has(${VISIBLE_UNFOCUSED_REGION}))`;

const PERIMETER_AFTER_BASE = {
  content: '""',
  position: "absolute",
  inset: 0,
  boxSizing: "border-box",
  border: "1px solid transparent",
  borderRadius: "inherit",
  pointerEvents: "none",
  zIndex: 3,
  boxShadow: "none",
  transition: "border-color 140ms ease, box-shadow 140ms ease",
};

const PERIMETER_AFTER_FOCUSED = {
  borderColor: desktopFocusEdgeColor,
  boxShadow: desktopRegionFocusBoxShadow,
  "@media (forced-colors: active)": {
    boxShadow: "none",
    border: "2px solid Highlight",
  },
};

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
      "&::after": PERIMETER_AFTER_BASE,
    },
    "& [data-desktop-region][data-desktop-focused='true']::after": PERIMETER_AFTER_FOCUSED,
    // Single-region panes promote the identical paint-only `::after` perimeter
    // onto the outer pane (enclosing its PaneHeader) and clear the inner region
    // overlay so no double seam forms directly below the header divider.
    [PROMOTABLE_DESKTOP_PANE_SELECTOR]: {
      position: "relative",
      "&::after": PERIMETER_AFTER_BASE,
    },
    [`${PROMOTED_DESKTOP_PANE_SELECTOR}::after`]: PERIMETER_AFTER_FOCUSED,
    [`${PROMOTED_DESKTOP_PANE_SELECTOR} [data-desktop-region][data-desktop-focused='true']::after`]: {
      borderColor: "transparent",
      boxShadow: "none",
      "@media (forced-colors: active)": {
        border: "1px solid transparent",
        boxShadow: "none",
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
