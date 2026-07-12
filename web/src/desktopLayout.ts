import { persisted, useStore } from "./_store/mod.ts";

// Desktop layout preference (DESKTOP-ONLY, persisted per device). Mobile/touch
// always uses "overlay" and ignores this. Same shape as composerExpand.ts.
//   - "overlay": the default single column — transcript full-bleed, the composer
//     floats over it at the bottom.
//   - "split": a two-column Zed-style layout — composer column (queued + drafts +
//     input) on the left, transcript on the right.
export type DesktopLayout = "overlay" | "split";

const layout = persisted<DesktopLayout>("cowboy:desktop-layout", "overlay", {
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
