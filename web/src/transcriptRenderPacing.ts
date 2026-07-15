// ACP can deliver text chunks much faster than a display needs to repaint.
// Keep the canonical store fully current, but let the presentation layer spend
// less of the main-thread budget while a reader is actively scrolling.
const IDLE_INTERVAL_MS = 50;
const SCROLLING_INTERVAL_MS = 100;
const SCROLL_ACTIVITY_WINDOW_MS = 240;

let scrollingUntil = 0;

export function markTranscriptScrollActivity(now = performance.now()): void {
  scrollingUntil = Math.max(scrollingUntil, now + SCROLL_ACTIVITY_WINDOW_MS);
}

export function transcriptPresentationIntervalMs(now = performance.now()): number {
  return now < scrollingUntil ? SCROLLING_INTERVAL_MS : IDLE_INTERVAL_MS;
}

export function resetTranscriptScrollActivityForTest(): void {
  scrollingUntil = 0;
}
