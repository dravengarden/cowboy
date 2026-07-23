export interface TranscriptViewportBackfillInput {
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
    input.reachedStart ||
    input.loadingOlder ||
    input.beforeSeq === null ||
    (input.desktop && input.fromResize)
  ) {
    return false;
  }
  return input.scrollHeight <= input.clientHeight + 1;
}
