import { reportClientLog } from "./observability";

const MIN_SWIPE_PX = 48;
const EDGE_PX = 2;
const REPORT_INTERVAL_MS = 60_000;

/** A deliberate vertical swipe that should have moved a scroller and did not.
 *  In a column-reverse scroller the newest edge is `scrollTop` 0 and older
 *  content is negative, so a finger moving down reveals older rows. */
export function swipeShouldHaveScrolled(sample: {
  deltaX: number;
  deltaY: number;
  scrollEvents: number;
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}): boolean {
  const { deltaX, deltaY, scrollEvents, scrollTop, scrollHeight, clientHeight } = sample;
  if (scrollEvents > 0) return false;
  if (Math.abs(deltaY) < MIN_SWIPE_PX) return false;
  if (Math.abs(deltaY) < Math.abs(deltaX) * 2) return false;
  const range = scrollHeight - clientHeight;
  if (range <= EDGE_PX) return false;
  const position = Math.abs(scrollTop);
  return deltaY > 0 ? position < range - EDGE_PX : position > EDGE_PX;
}

/** Report a Transcript that ignored a vertical swipe, with what was under the
 *  finger. Read-only and passive: it measures only after a gesture that did not
 *  scroll, so a normal swipe costs no layout. */
export function watchTranscriptScroll(
  scroller: HTMLElement,
  sessionId: string,
): () => void {
  let gesture: {
    x: number;
    y: number;
    lastX: number;
    lastY: number;
    scrollEvents: number;
    prevented: boolean;
    target: EventTarget | null;
  } | null = null;
  let lastReportAt = -REPORT_INTERVAL_MS;

  const onScroll = (): void => {
    if (gesture) gesture.scrollEvents += 1;
  };
  const onStart = (event: TouchEvent): void => {
    const touch = event.touches[0];
    gesture = event.touches.length === 1 && touch
      ? {
        x: touch.clientX,
        y: touch.clientY,
        lastX: touch.clientX,
        lastY: touch.clientY,
        scrollEvents: 0,
        prevented: false,
        target: event.target,
      }
      : null;
  };
  const onMove = (event: TouchEvent): void => {
    const touch = event.touches[0];
    if (!gesture || !touch) return;
    gesture.lastX = touch.clientX;
    gesture.lastY = touch.clientY;
    gesture.prevented ||= event.defaultPrevented;
  };
  const onEnd = (): void => {
    const finished = gesture;
    gesture = null;
    if (!finished || !scroller.isConnected) return;
    const deltaX = finished.lastX - finished.x;
    const deltaY = finished.lastY - finished.y;
    if (Math.abs(deltaY) < MIN_SWIPE_PX) return;
    const { scrollTop, scrollHeight, clientHeight } = scroller;
    if (
      !swipeShouldHaveScrolled({
        deltaX,
        deltaY,
        scrollEvents: finished.scrollEvents,
        scrollTop,
        scrollHeight,
        clientHeight,
      })
    ) return;
    const bounds = scroller.getBoundingClientRect();
    if (
      finished.x < bounds.left || finished.x > bounds.right ||
      finished.y < bounds.top || finished.y > bounds.bottom
    ) return;
    const hit = document.elementFromPoint(finished.x, finished.y);
    // An open rail or sheet legitimately owns this touch.
    if (hit?.closest("[data-detent-sheet='true'], [role='dialog']")) return;
    const now = performance.now();
    if (now - lastReportAt < REPORT_INTERVAL_MS) return;
    lastReportAt = now;
    const style = getComputedStyle(scroller);
    const target = finished.target instanceof Element ? finished.target : null;
    reportClientLog(
      "warn",
      "transcript_scroll_stalled",
      "Transcript ignored a vertical swipe",
      {
        session_id: sessionId,
        delta_y: Math.round(deltaY),
        scroll_top: Math.round(scrollTop),
        scroll_height: scrollHeight,
        client_height: clientHeight,
        overflow_y: style.overflowY,
        pointer_events: style.pointerEvents,
        visibility: style.visibility,
        touch_action: style.touchAction,
        prevented: finished.prevented,
        target_inside: target ? scroller.contains(target) : false,
        hit_inside: hit ? scroller.contains(hit) : false,
        hit_tag: hit?.tagName ?? "none",
        drawer_open: document.querySelector("[data-mobile-drawer-open]") !== null,
        drawer_moving: document.querySelector("[data-mobile-drawer-moving]") !== null,
        product_moving: document.querySelector("[data-mobile-product-moving]") !== null,
        render_paused: scroller.hasAttribute("data-render-paused"),
      },
    );
  };

  scroller.addEventListener("scroll", onScroll, { passive: true });
  const options = { capture: true, passive: true };
  document.addEventListener("touchstart", onStart, options);
  document.addEventListener("touchmove", onMove, options);
  document.addEventListener("touchend", onEnd, options);
  return () => {
    scroller.removeEventListener("scroll", onScroll);
    document.removeEventListener("touchstart", onStart, options);
    document.removeEventListener("touchmove", onMove, options);
    document.removeEventListener("touchend", onEnd, options);
  };
}
