export const BLANK_CANVAS_LONG_PRESS_MS = 520;
export const BLANK_CANVAS_MOVE_TOLERANCE_PX = 10;
export const BLANK_CANVAS_TEXT_BUFFER_PX = 14;

export interface BlankCanvasPressGeometry {
  clientY: number;
  textareaTop: number;
  textareaHeight: number;
  textareaScrollTop: number;
  naturalContentHeight: number;
  textBuffer?: number;
}

export interface BlankCanvasPressClaim extends BlankCanvasPressGeometry {
  expanded: boolean;
  disabled: boolean;
  touchCount: number;
}

/**
 * The iOS edit menu can only anchor reliably near laid-out text. A fullscreen
 * textarea is still the correct native editor, but the visually empty part
 * below its last rendered line needs Cowboy's non-modal Paste fallback.
 */
export function isBlankCanvasPress(
  geometry: BlankCanvasPressGeometry,
): boolean {
  const localY = geometry.clientY - geometry.textareaTop;
  if (localY < 0 || localY > geometry.textareaHeight) return false;

  const documentY = localY + geometry.textareaScrollTop;
  const buffer = geometry.textBuffer ?? BLANK_CANVAS_TEXT_BUFFER_PX;
  return documentY > geometry.naturalContentHeight + buffer;
}

/**
 * Claim only the part of a fullscreen touch editor where UIKit cannot place an
 * edit-menu anchor. Compact editors and real text stay entirely native.
 */
export function shouldClaimBlankCanvasPress(
  claim: BlankCanvasPressClaim,
): boolean {
  return claim.expanded && !claim.disabled && claim.touchCount === 1 &&
    isBlankCanvasPress(claim);
}

export function longPressMoved(
  originX: number,
  originY: number,
  currentX: number,
  currentY: number,
  tolerance = BLANK_CANVAS_MOVE_TOLERANCE_PX,
): boolean {
  return Math.hypot(currentX - originX, currentY - originY) > tolerance;
}

/** Measure the textarea's real wrapped text height without changing its value,
 * selection, first-responder status, or the visible layout. */
export function measureTextareaContentHeight(
  textarea: HTMLTextAreaElement,
): number {
  const mirror = textarea.cloneNode(false) as HTMLTextAreaElement;
  const rect = textarea.getBoundingClientRect();
  const computed = getComputedStyle(textarea);
  mirror.removeAttribute("id");
  mirror.removeAttribute("name");
  mirror.setAttribute("aria-hidden", "true");
  mirror.tabIndex = -1;
  mirror.value = textarea.value || textarea.placeholder || " ";
  mirror.style.setProperty("position", "fixed", "important");
  mirror.style.setProperty("inset", "auto auto auto -10000px", "important");
  mirror.style.setProperty("width", `${String(rect.width)}px`, "important");
  mirror.style.setProperty("height", "0", "important");
  mirror.style.setProperty("min-height", "0", "important");
  mirror.style.setProperty("max-height", "none", "important");
  mirror.style.setProperty("overflow", "hidden", "important");
  mirror.style.setProperty("visibility", "hidden", "important");
  mirror.style.setProperty("pointer-events", "none", "important");
  for (
    const property of [
      "box-sizing",
      "font-family",
      "font-size",
      "font-style",
      "font-weight",
      "letter-spacing",
      "line-height",
      "padding-top",
      "padding-right",
      "padding-bottom",
      "padding-left",
      "white-space",
      "word-break",
      "overflow-wrap",
      "tab-size",
    ]
  ) {
    mirror.style.setProperty(
      property,
      computed.getPropertyValue(property),
      "important",
    );
  }
  document.body.appendChild(mirror);
  const height = mirror.scrollHeight;
  mirror.remove();
  return height;
}
