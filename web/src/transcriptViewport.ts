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
