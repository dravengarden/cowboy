/** Review has two horizontal modes:
 *  - Wrap on, no horizontal bar: left/right is the workspace swipe.
 *    Content is viewport-wide, like README. Keep live CodeMirror.
 *  - Wrap off, horizontal bar: left/right pans the file. `hasHorizontalScroller`
 *    owns that gesture. Do not steal it for the drawer/pager.
 *
 *  Workspace swipe therefore only translates wrap-on source. Sticky
 *  gutters are unnecessary in that mode and are the remaining iOS
 *  re-raster. Do not hide the editor or snapshot it. */

export const MOBILE_CODE_SWIPE_START = "cowboy:transcript-direct-manipulation-start";
export const MOBILE_CODE_SWIPE_END = "cowboy:transcript-direct-manipulation-end";

export const mobileCodeRestLayerSx = {
  overflow: "hidden",
  isolation: "isolate",
  contain: "paint",
  backfaceVisibility: "hidden",
  WebkitBackfaceVisibility: "hidden",
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
    if (!local) return;
    local = false;
    freezeCount = Math.max(0, freezeCount - 1);
  };
  if (alreadyClaimed()) apply();
  globalThis.addEventListener(MOBILE_CODE_SWIPE_START, apply);
  globalThis.addEventListener(MOBILE_CODE_SWIPE_END, release);
  return () => {
    globalThis.removeEventListener(MOBILE_CODE_SWIPE_START, apply);
    globalThis.removeEventListener(MOBILE_CODE_SWIPE_END, release);
    release();
  };
}
