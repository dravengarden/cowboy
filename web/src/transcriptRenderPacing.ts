// ACP can deliver text chunks much faster than a display needs to repaint.
// Keep the canonical store fully current, but let the presentation layer spend
// less of the main-thread budget while a reader is actively scrolling. Touch
// surfaces deliberately paint streamed text less often: 10fps remains visually
// live on a phone while avoiding repeated Markdown/layout work that competes
// with the keyboard, scrolling and the device's thermal budget.
const DESKTOP_INTERVAL_MS = 50;
const DESKTOP_SCROLLING_INTERVAL_MS = 100;
const TOUCH_INTERVAL_MS = 100;
const TOUCH_SCROLLING_INTERVAL_MS = 150;
const SCROLL_ACTIVITY_WINDOW_MS = 240;

let scrollingUntil = 0;
let touchPresentation = false;

export function setTouchTranscriptPresentation(touch: boolean): void {
  touchPresentation = touch;
}

export function markTranscriptScrollActivity(now = performance.now()): void {
  scrollingUntil = Math.max(scrollingUntil, now + SCROLL_ACTIVITY_WINDOW_MS);
}

export function transcriptPresentationIntervalMs(now = performance.now()): number {
  if (touchPresentation) {
    return now < scrollingUntil ? TOUCH_SCROLLING_INTERVAL_MS : TOUCH_INTERVAL_MS;
  }
  return now < scrollingUntil
    ? DESKTOP_SCROLLING_INTERVAL_MS
    : DESKTOP_INTERVAL_MS;
}

export function resetTranscriptScrollActivityForTest(): void {
  scrollingUntil = 0;
  touchPresentation = false;
}
