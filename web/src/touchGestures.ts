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

export interface RetargetedTouchClickGuard {
  arm: () => void;
  reset: () => void;
  consume: (clickDetail: number, pointerType?: string) => boolean;
}

/**
 * Keep a completed touch gesture's compatibility click attached to that
 * gesture even when React reflows or replaces the original target before the
 * click arrives. A fresh pointerdown explicitly transfers ownership to the new
 * gesture; keyboard and assistive clicks never consume the touch claim.
 */
export function createRetargetedTouchClickGuard(): RetargetedTouchClickGuard {
  let pending = false;
  return {
    arm: (): void => {
      pending = true;
    },
    reset: (): void => {
      pending = false;
    },
    consume: (clickDetail: number, pointerType = ""): boolean => {
      const suppress = isPairedTouchClick(pending, clickDetail, pointerType);
      if (suppress) pending = false;
      return suppress;
    },
  };
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

/** Preserve native horizontal scrolling when a surface actually
 *  overflows on X. Wrap-on Review source uses `overflow-x: hidden`,
 *  so it falls through to the workspace swipe. Wrap-off source with a
 *  real bar keeps the pan. */
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

/** Keep native vertical scrolling authoritative inside a real overflow layer.
 *
 * A non-passive ancestor recognizer cannot safely "give the gesture back" after
 * one early horizontal sample has called preventDefault(): iOS cancels the
 * complete native pan at that point. Do not reserve workspace/drawer swipes
 * from an already-scrollable Y surface; surrounding chrome remains available
 * for horizontal navigation.
 */
export function hasVerticalScroller(
  target: EventTarget | null,
  boundary: HTMLElement,
): boolean {
  let node = target instanceof HTMLElement ? target : null;
  while (node && node !== boundary) {
    const style = globalThis.getComputedStyle(node);
    if (isVerticalScrollContainer(node, style.overflowY)) return true;
    node = node.parentElement;
  }
  return false;
}

export function isVerticalScrollContainer(
  geometry: Pick<HTMLElement, "clientHeight" | "scrollHeight">,
  overflowY: string,
): boolean {
  return geometry.scrollHeight > geometry.clientHeight + 2 &&
    (overflowY === "auto" || overflowY === "scroll");
}
