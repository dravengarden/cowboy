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

/** A `scroll` event alone does not prove reader intent. In a column-reverse
 * transcript, appending or growing content below a detached viewport makes the
 * browser compensate `scrollTop` and dispatch a trusted scroll event even
 * though the visible anchor did not move. Only an input-armed interaction may
 * take rendering ownership; layout compensation must remain presentation-live. */
export function transcriptScrollHasReaderIntent({
  nativeScrollActive,
  touchActive,
  pointerActive,
  directManipulationActive,
}: {
  nativeScrollActive: boolean;
  touchActive: boolean;
  pointerActive: boolean;
  directManipulationActive: boolean;
}): boolean {
  return nativeScrollActive || touchActive || pointerActive ||
    directManipulationActive;
}

/** Native keyboard commands that can move a focused transcript in either
 * direction. They arm scroll ownership before the browser emits `scroll`; the
 * narrower `keyLeavesLatest` predicate below still owns follow-mode changes. */
export function keyMovesTranscriptViewport(
  event: Pick<KeyboardEvent, "key">,
): boolean {
  return event.key === "ArrowUp" ||
    event.key === "ArrowDown" ||
    event.key === "PageUp" ||
    event.key === "PageDown" ||
    event.key === "Home" ||
    event.key === "End" ||
    event.key === " ";
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
