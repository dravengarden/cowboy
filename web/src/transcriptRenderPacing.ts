// ACP can deliver text chunks much faster than a display needs to repaint.
// Keep the canonical store fully current, but let the presentation layer spend
// less of the main-thread budget while a reader is actively scrolling.
const IDLE_INTERVAL_MS = 50;
const SCROLLING_INTERVAL_MS = 100;
const SCROLL_ACTIVITY_WINDOW_MS = 240;
const GEOMETRY_INTERVAL_MS = 96;

let scrollingUntil = 0;

export function markTranscriptScrollActivity(now = performance.now()): void {
  scrollingUntil = Math.max(scrollingUntil, now + SCROLL_ACTIVITY_WINDOW_MS);
}

export function transcriptPresentationIntervalMs(now = performance.now()): number {
  return now < scrollingUntil ? SCROLLING_INTERVAL_MS : IDLE_INTERVAL_MS;
}

// A composer disclosure animates its height every frame. The glass follows that
// motion live, but applying every intermediate height as transcript padding lays
// out the entire retained column-reverse history. Return the delay until the next
// geometry publication so App can coalesce those expensive padding changes while
// still publishing the final height after the transition settles.
export function transcriptGeometryDelayMs(now: number, lastPublishedAt: number): number {
  if (lastPublishedAt === 0) return 0;
  return Math.max(0, GEOMETRY_INTERVAL_MS - (now - lastPublishedAt));
}

export function resetTranscriptScrollActivityForTest(): void {
  scrollingUntil = 0;
}
