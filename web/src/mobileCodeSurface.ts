/** Review has two horizontal modes:
 *  - Wrap on, no horizontal bar: left/right is the workspace swipe.
 *    Content is viewport-wide, like README. Keep live CodeMirror.
 *  - Wrap off, horizontal bar: left/right pans the file. `hasHorizontalScroller`
 *    owns that gesture. Do not steal it for the drawer/pager.
 *
 *  iOS re-rasters CM token spans under an ancestor translate3d. Overflow
 *  flatten remasures the scroller. Hiding flashes. A canvas snapshot was
 *  rejected. During a claimed swipe, `filter: opacity(0.999)` forces one
 *  compositor texture of the live editor without changing overflow. */

export const MOBILE_CODE_SWIPE_START = "cowboy:transcript-direct-manipulation-start";
export const MOBILE_CODE_SWIPE_END = "cowboy:transcript-direct-manipulation-end";

export const mobileCodeRestLayerSx = {
  overflow: "hidden",
  isolation: "isolate",
  contain: "paint",
  backfaceVisibility: "hidden",
  WebkitBackfaceVisibility: "hidden",
  // Inner CM pieces must not self-promote. A nested overflow tile plus
  // sticky gutters is what iOS relocates every swipe frame.
  "& .cm-scroller, & .cm-content, & .cm-gutters, & .cm-layer": {
    backfaceVisibility: "visible",
    isolation: "auto",
    transform: "none",
    willChange: "auto",
    WebkitOverflowScrolling: "auto",
  },
} as const;

/** Claimed-swipe flatten. Opacity 0.999 so WebKit cannot skip the filter. */
export const mobileCodeSwipeFlattenSx = {
  "& [data-mobile-code-layer]": {
    filter: "opacity(0.999)",
    WebkitFilter: "opacity(0.999)",
    willChange: "transform",
  },
} as const;

let freezeCount = 0;

export function isMobileCodeSwipeFrozen(): boolean {
  return freezeCount > 0;
}

export function swipeOwnsCodeSurface(
  query: ((selector: string) => unknown) | undefined = globalThis.document
    ?.querySelector.bind(globalThis.document),
): boolean {
  return query?.(
    "[data-mobile-drawer-moving='true'], " +
      "[data-mobile-product-moving='true'], " +
      "[data-mobile-sheet-presented='true']",
  ) != null;
}

export function bindCodeViewerSwipeFreeze(
  _view: {
    scrollDOM: HTMLElement;
    dom: HTMLElement;
  },
  alreadyClaimed: () => boolean = swipeOwnsCodeSurface,
): () => void {
  let local = false;
  const apply = (): void => {
    if (local) return;
    local = true;
    freezeCount += 1;
  };
  const release = (): void => {
    if (local && swipeOwnsCodeSurface()) return;
    if (!local) return;
    local = false;
    freezeCount = Math.max(0, freezeCount - 1);
  };
  const onTouchStart = (): void => {
    apply();
  };
  const onTouchEnd = (): void => {
    release();
  };
  if (alreadyClaimed()) apply();
  globalThis.addEventListener("touchstart", onTouchStart, {
    capture: true,
    passive: true,
  });
  globalThis.addEventListener("touchend", onTouchEnd, { passive: true });
  globalThis.addEventListener("touchcancel", onTouchEnd, { passive: true });
  globalThis.addEventListener(MOBILE_CODE_SWIPE_START, apply);
  globalThis.addEventListener(MOBILE_CODE_SWIPE_END, release);
  return () => {
    globalThis.removeEventListener("touchstart", onTouchStart, true);
    globalThis.removeEventListener("touchend", onTouchEnd);
    globalThis.removeEventListener("touchcancel", onTouchEnd);
    globalThis.removeEventListener(MOBILE_CODE_SWIPE_START, apply);
    globalThis.removeEventListener(MOBILE_CODE_SWIPE_END, release);
    if (!local) return;
    local = false;
    freezeCount = Math.max(0, freezeCount - 1);
  };
}
