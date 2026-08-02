export interface TranscriptViewportBackfillInput {
  managed: boolean;
  allowed: boolean;
  desktop: boolean;
  fromResize: boolean;
  reachedStart: boolean;
  loadingOlder: boolean;
  beforeSeq: number | null;
  scrollHeight: number;
  clientHeight: number;
  loadingFillHeight: number | null;
}

export function shouldBackfillTranscriptViewport(
  input: TranscriptViewportBackfillInput,
): boolean {
  if (
    !input.managed ||
    !input.allowed ||
    input.reachedStart ||
    input.loadingOlder ||
    input.beforeSeq === null ||
    (input.desktop && input.fromResize)
  ) {
    return false;
  }
  // Once mounted, the flex filler is the exact unused reading area. Ignore a
  // tiny remainder so fractional layout and safe-area rounding cannot create a
  // load/stop oscillation. Before it mounts, a non-overflowing scroll box is
  // enough to start the first small request and reveal the filler.
  if (input.loadingFillHeight !== null) return input.loadingFillHeight > 16;
  return input.scrollHeight <= input.clientHeight + 1;
}

export interface HistoryPrefetchInput {
  managed: boolean;
  detached: boolean;
  armed: boolean;
  fromTop: number;
  threshold: number;
}

export function historyPrefetchTransition(
  input: HistoryPrefetchInput,
): { armed: boolean; request: boolean } {
  if (!input.managed) return { armed: false, request: false };
  if (!input.detached) return { armed: true, request: false };
  if (input.fromTop >= input.threshold) return { armed: true, request: false };
  if (input.armed) return { armed: false, request: true };
  return { armed: false, request: false };
}

export function scrollbackFillRemaining(input: {
  targetHeight: number;
  baseScrollHeight: number;
  currentScrollHeight: number;
  skeletonHeight: number;
}): number {
  const addedHeight = Math.max(
    0,
    input.currentScrollHeight - input.baseScrollHeight - input.skeletonHeight,
  );
  return Math.max(0, input.targetHeight - addedHeight);
}

export function shouldContinueScrollbackFill(input: {
  remaining: number;
  loadedRows: number;
  minimumRows: number;
  fromTop: number;
  viewportHeight: number;
  reachedStart: boolean;
  loadingOlder: boolean;
  beforeSeq: number | null;
}): boolean {
  return !input.reachedStart &&
    !input.loadingOlder &&
    input.beforeSeq !== null &&
    input.fromTop <= input.viewportHeight * 3 &&
    (input.loadedRows < input.minimumRows || input.remaining > 24);
}

export interface TranscriptMagnetInput {
  history: boolean;
  working: boolean;
  detached: boolean;
  touching: boolean;
  fromBottom: number;
  threshold: number;
}

/** History always owns bottom-following. Page View only owns it while its live
 * tail is actively streaming; a settled page's control remains a plain
 * scroll-to-page-bottom action. */
export function shouldMagnetizeTranscript(
  input: TranscriptMagnetInput,
): boolean {
  return (input.history || input.working) &&
    input.detached &&
    !input.touching &&
    input.fromBottom <= input.threshold;
}

export function magneticHapticTransition(
  armed: boolean,
  fromBottom: number,
  threshold: number,
): { armed: boolean; fire: boolean } {
  if (!armed && fromBottom <= threshold) return { armed: true, fire: true };
  // Native scroll settling can oscillate by a few pixels at the boundary.
  // Require a deliberate retreat before another quiet orientation tick.
  if (armed && fromBottom >= threshold * 1.75) {
    return { armed: false, fire: false };
  }
  return { armed, fire: false };
}
