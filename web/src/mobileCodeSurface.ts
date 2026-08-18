/** Code Review's read-only CodeMirror is a nested iOS overflow tile.
 *  CSS flatten can lose to CM theme modules after a lazy grammar load;
 *  inline styles on `scrollDOM` always win. */

export const MOBILE_CODE_SWIPE_START = "cowboy:transcript-direct-manipulation-start";
export const MOBILE_CODE_SWIPE_END = "cowboy:transcript-direct-manipulation-end";

let freezeCount = 0;

export function isMobileCodeSwipeFrozen(): boolean {
  return freezeCount > 0;
}

export function freezeMobileOverflowTile(scroller: HTMLElement): void {
  scroller.style.setProperty("-webkit-overflow-scrolling", "auto", "important");
  scroller.style.setProperty("overflow", "hidden", "important");
  scroller.style.setProperty("overflow-x", "hidden", "important");
  scroller.style.setProperty("contain", "paint");
  scroller.style.setProperty("pointer-events", "none");
}

export function thawMobileOverflowTile(scroller: HTMLElement): void {
  scroller.style.removeProperty("-webkit-overflow-scrolling");
  scroller.style.removeProperty("overflow");
  scroller.style.removeProperty("overflow-x");
  scroller.style.removeProperty("contain");
  scroller.style.removeProperty("pointer-events");
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
  view: {
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
    view.dom.setAttribute("data-mobile-code-frozen", "true");
    freezeMobileOverflowTile(view.scrollDOM);
  };
  const release = (): void => {
    if (!local) return;
    local = false;
    freezeCount = Math.max(0, freezeCount - 1);
    view.dom.removeAttribute("data-mobile-code-frozen");
    thawMobileOverflowTile(view.scrollDOM);
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
