export type HorizontalDirection = "left" | "right";

export interface HorizontalSwipe {
  direction: HorizontalDirection;
  distance: number;
}

/** Lock a touch gesture to the horizontal axis only after intent is clear. */
export function horizontalSwipe(
  deltaX: number,
  deltaY: number,
  lockDistance = 12,
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
