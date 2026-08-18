/** Review CodeMirror cannot ride a workspace translate.
 *  Overflow flatten remasures. Hiding the editor flashes. A standing
 *  `translateZ(0)` tile still re-rasters token spans under iOS contain.
 *  The swipe path therefore translates a bitmap of the current viewport
 *  that is painted on finger-down, before the first `translate3d`. */

export const MOBILE_CODE_SWIPE_START = "cowboy:transcript-direct-manipulation-start";
export const MOBILE_CODE_SWIPE_END = "cowboy:transcript-direct-manipulation-end";

export const mobileCodeRestLayerSx = {
  position: "relative",
  overflow: "hidden",
  isolation: "isolate",
  contain: "paint",
  backfaceVisibility: "hidden",
  WebkitBackfaceVisibility: "hidden",
} as const;

export const mobileCodeSnapshotHostSx = {
  position: "relative",
  "& [data-mobile-code-snapshot]": {
    position: "absolute",
    inset: 0,
    width: "100%",
    height: "100%",
    pointerEvents: "none",
    visibility: "hidden",
  },
  "&[data-mobile-code-frozen] [data-mobile-code-snapshot]": {
    visibility: "visible",
  },
  "&[data-mobile-code-frozen] .cm-editor": {
    visibility: "hidden",
  },
} as const;

export const mobileCodeSwipeSwapSx = {
  "& [data-mobile-code-layer] [data-mobile-code-snapshot]": {
    visibility: "visible",
  },
  "& [data-mobile-code-layer] .cm-editor": {
    visibility: "hidden",
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

function paintBox(
  ctx: CanvasRenderingContext2D,
  el: Element,
  origin: DOMRect,
  fill: string,
): void {
  const box = el.getBoundingClientRect();
  if (box.width <= 0 || box.height <= 0) return;
  if (box.bottom < origin.top || box.top > origin.bottom) return;
  ctx.fillStyle = fill;
  ctx.fillRect(
    box.left - origin.left,
    box.top - origin.top,
    box.width,
    box.height,
  );
}

function paintTextNode(
  ctx: CanvasRenderingContext2D,
  node: Text,
  origin: DOMRect,
): void {
  const text = node.data;
  if (!text || !text.trim() && text !== " ") return;
  const parent = node.parentElement;
  if (!parent) return;
  const range = parent.ownerDocument?.createRange();
  if (!range) return;
  range.selectNodeContents(node);
  const box = range.getBoundingClientRect();
  if (box.width <= 0 || box.height <= 0) return;
  if (box.bottom < origin.top || box.top > origin.bottom) return;
  const style = globalThis.getComputedStyle(parent);
  ctx.fillStyle = style.color;
  ctx.font = style.font;
  ctx.textBaseline = "top";
  ctx.fillText(text, box.left - origin.left, box.top - origin.top);
}

function paintTree(
  ctx: CanvasRenderingContext2D,
  root: Element,
  origin: DOMRect,
): void {
  const walk = (node: Node): void => {
    if (node.nodeType === 3) {
      paintTextNode(ctx, node as Text, origin);
      return;
    }
    if (node.nodeType !== 1) return;
    for (const child of node.childNodes) walk(child);
  };
  walk(root);
}

/** Raster the visible CodeMirror viewport into `canvas`. Called on
 *  finger-down / idle, never on touchmove. */
export function paintCodeViewportFromDom(
  scroller: HTMLElement,
  canvas: HTMLCanvasElement,
): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const origin = scroller.getBoundingClientRect();
  const width = Math.max(1, Math.round(origin.width));
  const height = Math.max(1, Math.round(origin.height));
  const dpr = Math.max(1, globalThis.devicePixelRatio || 1);
  if (canvas.width !== width * dpr || canvas.height !== height * dpr) {
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    canvas.style.width = `${String(width)}px`;
    canvas.style.height = `${String(height)}px`;
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const scrollerStyle = globalThis.getComputedStyle(scroller);
  ctx.fillStyle = scrollerStyle.backgroundColor || "transparent";
  ctx.fillRect(0, 0, width, height);
  const gutters = scroller.querySelector(".cm-gutters");
  if (gutters instanceof HTMLElement) {
    paintBox(
      ctx,
      gutters,
      origin,
      globalThis.getComputedStyle(gutters).backgroundColor ||
        scrollerStyle.backgroundColor,
    );
  }
  for (const line of scroller.querySelectorAll(".cm-line")) {
    if (!(line instanceof HTMLElement)) continue;
    const bg = globalThis.getComputedStyle(line).backgroundColor;
    if (bg && bg !== "rgba(0, 0, 0, 0)" && bg !== "transparent") {
      paintBox(ctx, line, origin, bg);
    }
    paintTree(ctx, line, origin);
  }
  for (const mark of scroller.querySelectorAll(".cm-gutterElement")) {
    if (mark instanceof HTMLElement) paintTree(ctx, mark, origin);
  }
}

export function bindCodeViewerSwipeFreeze({
  view,
  layer,
  canvas,
  paint = paintCodeViewportFromDom,
}: {
  view: { scrollDOM: HTMLElement; dom: HTMLElement };
  layer: HTMLElement;
  canvas: HTMLCanvasElement;
  paint?: (scroller: HTMLElement, canvas: HTMLCanvasElement) => void;
}): () => void {
  let local = false;
  let dirty = true;
  const capture = (): void => {
    if (!dirty) return;
    paint(view.scrollDOM, canvas);
    dirty = false;
  };
  const markDirty = (): void => {
    dirty = true;
  };
  const apply = (): void => {
    capture();
    if (local) return;
    local = true;
    freezeCount += 1;
    layer.setAttribute("data-mobile-code-frozen", "true");
  };
  const release = (): void => {
    if (!local) return;
    local = false;
    freezeCount = Math.max(0, freezeCount - 1);
    layer.removeAttribute("data-mobile-code-frozen");
  };
  const onTouchStart = (): void => {
    capture();
  };
  view.scrollDOM.addEventListener("scroll", markDirty, { passive: true });
  globalThis.addEventListener("touchstart", onTouchStart, {
    capture: true,
    passive: true,
  });
  capture();
  if (swipeOwnsCodeSurface()) apply();
  globalThis.addEventListener(MOBILE_CODE_SWIPE_START, apply);
  globalThis.addEventListener(MOBILE_CODE_SWIPE_END, release);
  return () => {
    view.scrollDOM.removeEventListener("scroll", markDirty);
    globalThis.removeEventListener("touchstart", onTouchStart, true);
    globalThis.removeEventListener(MOBILE_CODE_SWIPE_START, apply);
    globalThis.removeEventListener(MOBILE_CODE_SWIPE_END, release);
    release();
  };
}
