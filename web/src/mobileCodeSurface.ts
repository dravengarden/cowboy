/** Review CodeMirror must leave the transforming peek. Changing
 *  `.cm-scroller` overflow remasures the document and destroys iOS's
 *  touch-scroll cache on the first tracking frame — that was the failed
 *  flatten. Hide paint only; keep layout and overflow alone. */

export const MOBILE_CODE_SWIPE_START = "cowboy:transcript-direct-manipulation-start";
export const MOBILE_CODE_SWIPE_END = "cowboy:transcript-direct-manipulation-end";

export const mobileCodePaintCullSx = {
  visibility: "hidden",
  pointerEvents: "none",
} as const;

let freezeCount = 0;

export function isMobileCodeSwipeFrozen(): boolean {
  return freezeCount > 0;
}

export function hideMobileCodePaint(layer: HTMLElement): void {
  layer.style.visibility = "hidden";
  layer.style.pointerEvents = "none";
}

export function showMobileCodePaint(layer: HTMLElement): void {
  layer.style.removeProperty("visibility");
  layer.style.removeProperty("pointer-events");
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
  const closest = view.dom.closest("[data-mobile-code-layer]");
  const layer = closest != null && "style" in closest
    ? closest as HTMLElement
    : view.dom;
  let local = false;
  const apply = (): void => {
    if (local) return;
    local = true;
    freezeCount += 1;
    layer.setAttribute("data-mobile-code-frozen", "true");
    hideMobileCodePaint(layer);
  };
  const release = (): void => {
    if (!local) return;
    local = false;
    freezeCount = Math.max(0, freezeCount - 1);
    layer.removeAttribute("data-mobile-code-frozen");
    showMobileCodePaint(layer);
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
