/** Review CodeMirror stays visible during a workspace swipe.
 *  Changing `.cm-scroller` overflow remasures and hitchs. Hiding the
 *  layer flashes a blank pane. The standing contract is a viewport-sized
 *  compositor layer at rest; swipe JS only skips measure callbacks. */

export const MOBILE_CODE_SWIPE_START = "cowboy:transcript-direct-manipulation-start";
export const MOBILE_CODE_SWIPE_END = "cowboy:transcript-direct-manipulation-end";

/** Clip and promote the editor to one phone-sized tile. No
 *  `-webkit-overflow-scrolling: touch` on the inner scroller, or iOS
 *  still owns a max-content overflow texture. */
export const mobileCodeRestLayerSx = {
  overflow: "hidden",
  isolation: "isolate",
  contain: "paint",
  backfaceVisibility: "hidden",
  WebkitBackfaceVisibility: "hidden",
  transform: "translate3d(0, 0, 0)",
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
