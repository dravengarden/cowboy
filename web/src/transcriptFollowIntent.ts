/**
 * User inputs that move a conventional vertical viewport away from its latest
 * (bottom) edge. `WheelEvent.deltaY` already reflects the OS scroll direction,
 * including trackpad natural-scrolling preferences; only its sign matters.
 */
export function wheelLeavesLatest(deltaY: number): boolean {
  return deltaY < 0;
}

/** A detached reader owns scrollTop while a native scroll gesture is active.
 * Replaying the streaming-content anchor during that window competes with
 * touch momentum, wheel inertia, or scrollbar dragging and makes the viewport
 * oscillate. This rule is deliberately shared by Mobile and Desktop. */
export function shouldRestoreDetachedAnchor(
  _desktopNavigation: boolean,
  nativeScrollActive: boolean,
): boolean {
  return !nativeScrollActive;
}

/** Keyboard scroll commands that move away from the latest (bottom) edge.
 * Down/End/Space are deliberately excluded: their inertial/key-repeat tail may
 * continue after the viewport reaches the bottom and must not turn Following
 * straight back off. */
export function keyLeavesLatest(event: Pick<KeyboardEvent, "key" | "shiftKey">): boolean {
  return event.key === "ArrowUp" ||
    event.key === "PageUp" ||
    event.key === "Home" ||
    (event.key === " " && event.shiftKey);
}
