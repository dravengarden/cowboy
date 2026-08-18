export type HorizontalDirection = "left" | "right";

export interface HorizontalSwipe {
  direction: HorizontalDirection;
  distance: number;
}

// A stationary row tap must stop qualifying before the drawer can acquire the
// same touch stream. Keeping a real dead band between these thresholds prevents
// pointerup from selecting a session only for touchend to replace that pending
// selection with a drawer settle.
export const RELIABLE_TOUCH_TAP_MOVE_SLOP_PX = 10;
/** Generic horizontal recognizer slop. The Sessions/Review drawer uses
 *  `obsidianDrawerGesture` instead of these defaults. */
export const MOBILE_DRAWER_DIRECTION_LOCK_PX = 12;
export const MOBILE_DRAWER_PREPARE_PX = 4;

/**
 * A click synthesized from a completed touch reports touch pointer ownership or
 * a non-zero click count. Keep the pairing pending until that click actually
 * arrives: iOS WebKit may delay it well past a fixed suppression timeout.
 * Keyboard and assistive activations use detail=0 and must remain native even
 * if no paired click ever arrived.
 */
export function isPairedTouchClick(
  pendingTouchClick: boolean,
  clickDetail: number,
  pointerType = "",
): boolean {
  return pendingTouchClick && (pointerType === "touch" || clickDetail !== 0);
}

/** Lock a touch gesture to the horizontal axis only after intent is clear. */
export function horizontalSwipe(
  deltaX: number,
  deltaY: number,
  lockDistance = MOBILE_DRAWER_DIRECTION_LOCK_PX,
  dominance = 1.35,
): HorizontalSwipe | null {
  const distance = Math.abs(deltaX);
  if (distance < lockDistance || distance < Math.abs(deltaY) * dominance) return null;
  return { direction: deltaX < 0 ? "left" : "right", distance };
}

/** Release an ancestor horizontal recognizer once vertical intent is clear. */
export function isDominantVerticalPan(
  deltaX: number,
  deltaY: number,
  lockDistance = 10,
  dominance = 1.15,
): boolean {
  return Math.abs(deltaY) >= lockDistance &&
    Math.abs(deltaY) > Math.abs(deltaX) * dominance;
}

export function shouldFreezePreviewPointer(
  pointerType: string,
  button: number,
): boolean {
  return pointerType === "mouse" && button === 0;
}

export function swipeCommits(distance: number, viewportWidth: number): boolean {
  return distance >= Math.min(112, Math.max(88, viewportWidth * 0.24));
}

/** Only a real keyboard-owned floating editor reserves the shell swipe. */
export function inputOverlayOwnsDrawerGesture(
  keyboardOpen: boolean,
  focusWithin: boolean,
): boolean {
  return keyboardOpen && focusWithin;
}

/** Native selection-handle drags own the touch stream until the range collapses. */
export function expandedSelection(
  selection: Pick<Selection, "isCollapsed" | "rangeCount"> | null,
): boolean {
  return selection !== null && selection.rangeCount > 0 && !selection.isCollapsed;
}

/** Preserve native horizontal scrolling inside genuinely overflowing
 *  regions. Review CodeMirror is excluded: wrap-off source is
 *  `width: max-content`, so treating it as a scroller steals the
 *  workspace swipe. Horizontal reading uses the wrap toggle. */
export function hasHorizontalScroller(target: EventTarget | null, boundary: HTMLElement): boolean {
  let node = target instanceof HTMLElement ? target : null;
  if (node?.closest("[data-mobile-code-layer]")) return false;
  while (node && node !== boundary) {
    const style = globalThis.getComputedStyle(node);
    if (
      node.scrollWidth > node.clientWidth + 2 &&
      (style.overflowX === "auto" || style.overflowX === "scroll")
    ) return true;
    node = node.parentElement;
  }
  return false;
}
