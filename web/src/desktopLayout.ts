import { persisted, useStore } from "./_store/mod.ts";

// Desktop layout preference (DESKTOP-ONLY, persisted per device). Mobile/touch
// always uses "overlay" and ignores this. Same shape as composerExpand.ts.
//   - "split": the Desktop default — composer column (queued + drafts + input)
//     on the left, transcript on the right. With the Sessions rail this is the
//     three-column production workspace.
//   - "overlay": an explicit distraction-free escape hatch; never selected by
//     a viewport breakpoint and never shared with Mobile's layout preference.
export type DesktopLayout = "overlay" | "split";

// v2 intentionally migrates the old overlay-by-default era. Desktop and Mobile
// have since become separate products, so every Desktop installation gets the
// productivity-first split baseline once; subsequent user choices persist.
const layout = persisted<DesktopLayout>("cowboy:desktop-layout-v2", "split", {
  serialize: (v) => v,
  deserialize: (s) => (s === "split" ? "split" : "overlay"),
});

export function useDesktopLayout(): DesktopLayout {
  return useStore(layout);
}

export function setDesktopLayout(v: DesktopLayout): void {
  layout.set(v);
}

// Width (px) of the composer column in split mode — persisted, clamped. Global
// (like the session sidebar width), not per-session.
export const COMPOSER_COL_MIN = 320;
export const COMPOSER_COL_MAX = 720;
const COMPOSER_COL_DEFAULT = 440;

function clampColWidth(px: number): number {
  return Math.min(COMPOSER_COL_MAX, Math.max(COMPOSER_COL_MIN, Math.round(px)));
}

// Raw store exported so the splitter drag can mirror the sidebar-resize pattern
// (seed from `.get`, persist on pointerup via `.set`) — per-pixel `.set` during a
// drag would thrash localStorage, so the drag keeps a local value and commits once.
export const composerColWidthStore = persisted<number>(
  "cowboy:composer-col-width",
  COMPOSER_COL_DEFAULT,
  {
    serialize: (n) => String(Math.round(n)),
    deserialize: (s) => {
      const n = Number(s);
      return Number.isFinite(n) ? clampColWidth(n) : COMPOSER_COL_DEFAULT;
    },
  },
);

export function useComposerColWidth(): number {
  return useStore(composerColWidthStore);
}

export function clampComposerColWidth(px: number): number {
  return clampColWidth(px);
}
