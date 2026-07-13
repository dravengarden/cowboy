import { persisted, useStore } from "./_store/mod.ts";

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
