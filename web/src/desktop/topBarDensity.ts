// Desktop top-bar information density. Pure so the geometry decisions are
// testable without a DOM, and so the strip has ONE place that decides what it
// shows at a given width.
//
// The rule the whole module serves: the toolbar must never push a control out
// of the viewport. Horizontal scrolling looked cheap but it hides capability —
// reaching Clear meant scrolling a 4px bar. Instead the strip degrades in a
// fixed order, and only presentations that keep the fact (tooltip, visible
// keycap, Command Palette entry) are allowed to collapse.

/** Quota remaining, bucketed for colour. A number alone is not scannable: 0%
 *  and 95% render identically until one of them is red. */
export type UsageTone = "critical" | "low" | "normal";

export function usageRemainingTone(remaining: number): UsageTone {
  if (remaining <= 10) return "critical";
  if (remaining <= 25) return "low";
  return "normal";
}

/** Time-to-reset in the narrowest form that still answers "when?".
 *  Returns undefined when the account reports no reset. */
export function usageCountdown(
  resetsAtSeconds: number | undefined,
  now = Date.now(),
): string | undefined {
  if (resetsAtSeconds === undefined) return undefined;
  const minutes = Math.max(
    0,
    Math.ceil((resetsAtSeconds * 1000 - now) / 60_000),
  );
  if (minutes === 0) return "now";
  if (minutes < 60) return `${String(minutes)}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const rest = minutes % 60;
    return rest === 0
      ? `${String(hours)}h`
      : `${String(hours)}h${String(rest)}m`;
  }
  const days = Math.floor(hours / 24);
  const rest = hours % 24;
  return rest === 0 ? `${String(days)}d` : `${String(days)}d${String(rest)}h`;
}

/** One quota segment at full density: `Anthropic 95%` over `Weekly · 3d20h`.
 *  The old 156px was set by an absolute stamp (`resets Sep 20 02:00 PM`) that
 *  the countdown replaces — same fact, and the absolute form still lives in the
 *  U panel, which shows both. */
export const USAGE_SEGMENT_WIDTH_PX = 104;
/** The widest a balance account (DeepSeek) may grow: spend, partial-pricing,
 *  cache-miss and blocking-error counters. It hugs shorter content and
 *  truncates beyond this, so the budget is an upper bound, not a fixed cell. */
export const USAGE_BALANCE_SEGMENT_WIDTH_PX = 220;
/** Icon + visible keycap, with the word in the tooltip and the palette. */
export const ACTION_ICON_WIDTH_PX = 62;

export type TopBarDensity = "full" | "compact";

export interface TopBarWidths {
  /** Run configuration summary — never collapses; it is the densest control. */
  readonly config: number;
  /** The whole quota group including its U keycap. */
  readonly usage: number;
  /** Reload / Compact / Clear with their words. */
  readonly actions: number;
  /** Same cluster with words dropped to tooltips. */
  readonly compactActions: number;
  /** Region keycap, divider, Settings, Code — outside this component. */
  readonly trailing: number;
}

export function topBarWidth(
  widths: TopBarWidths,
  density: TopBarDensity,
): number {
  const actions = density === "full" ? widths.actions : widths.compactActions;
  return widths.config + widths.usage + actions + widths.trailing;
}

/** Widen before narrowing: re-expanding needs to clear the collapse threshold
 *  by a margin, or a toolbar parked exactly on the boundary flickers between
 *  densities while the user drags a splitter. */
export const TOPBAR_DENSITY_HYSTERESIS_PX = 28;

export function topBarDensity(
  available: number,
  widths: TopBarWidths,
  current: TopBarDensity = "full",
): TopBarDensity {
  // Before the first measurement lands, stay full: the pane is usually wide
  // enough, and starting compact would flash the words in on mount.
  if (!Number.isFinite(available) || available <= 0) return current;
  if (current === "full") {
    return available >= topBarWidth(widths, "full") ? "full" : "compact";
  }
  return available >= topBarWidth(widths, "full") + TOPBAR_DENSITY_HYSTERESIS_PX
    ? "full"
    : "compact";
}
