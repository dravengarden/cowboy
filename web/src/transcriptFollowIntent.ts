/**
 * User inputs that move a conventional vertical viewport away from its latest
 * (bottom) edge. `WheelEvent.deltaY` already reflects the OS scroll direction,
 * including trackpad natural-scrolling preferences; only its sign matters.
 * Horizontal-dominant trackpad pans (for example scrolling a wide code block or
 * table sideways with a small diagonal `deltaY` drift) must not detach live
 * transcript following.
 */
export function wheelLeavesLatest(deltaY: number, deltaX = 0): boolean {
  return deltaY < 0 && Math.abs(deltaY) >= Math.abs(deltaX);
}

/** True when a descendant inside `boundary` can itself consume a vertical
 *  wheel gesture in the requested direction (for example an expanded error
 *  `<pre>` or retained compaction summary). Letting that nested scroll run
 *  must not detach the outer transcript from its live edge. */
export function canNestedElementScrollVertically(
  target: EventTarget | null,
  boundary: HTMLElement,
  deltaY: number,
): boolean {
  if (deltaY === 0 || !(target instanceof Element)) return false;
  const view = boundary.ownerDocument.defaultView;
  if (!view) return false;
  let node: Element | null = target;
  while (node && node !== boundary) {
    if (node instanceof HTMLElement) {
      const overflowY = view.getComputedStyle(node).overflowY;
      if (
        (overflowY === "auto" || overflowY === "scroll" || overflowY === "overlay") &&
        node.scrollHeight > node.clientHeight + 1
      ) {
        if (deltaY < 0 && node.scrollTop > 0.5) return true;
        if (deltaY > 0 && node.scrollTop + node.clientHeight < node.scrollHeight - 0.5) {
          return true;
        }
      }
    }
    node = node.parentElement;
  }
  return false;
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

/** Whether reaching the bottom edge (for example via keyboard `j`/`Ctrl-D`)
 *  should re-enable Following. History always follows at bottom; Explore Page
 *  View only follows while the tail page is actively streaming. */
export function shouldFollowAtBottomEdge(
  managesScrollHistory: boolean,
  working: boolean,
): boolean {
  return managesScrollHistory || working;
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
