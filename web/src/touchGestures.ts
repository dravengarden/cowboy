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
export const MOBILE_DRAWER_DIRECTION_LOCK_PX = 12;

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

export function shouldFreezePreviewPointer(
  pointerType: string,
  button: number,
): boolean {
  return pointerType === "mouse" && button === 0;
}

export function swipeCommits(distance: number, viewportWidth: number): boolean {
  return distance >= Math.min(112, Math.max(88, viewportWidth * 0.24));
}

/** Native selection-handle drags own the touch stream until the range collapses. */
export function expandedSelection(
  selection: Pick<Selection, "isCollapsed" | "rangeCount"> | null,
): boolean {
  return selection !== null && selection.rangeCount > 0 && !selection.isCollapsed;
}

/** Preserve native horizontal scrolling inside genuinely overflowing code. */
export function hasHorizontalScroller(target: EventTarget | null, boundary: HTMLElement): boolean {
  let node = target instanceof HTMLElement ? target : null;
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
