// Syntax highlighting is presentation, not the source of truth. Prism emits a
// span for most tokens; very large tool payloads can therefore turn a compact,
// visually-clipped preview into tens of thousands of live DOM nodes. Keep the
// exact text but use the lightweight renderer once either dimension becomes
// pathological for a mobile sheet.
export const HIGHLIGHT_CHAR_LIMIT = 50_000;
export const HIGHLIGHT_LINE_LIMIT = 1_200;
export const LIGHTWEIGHT_CHUNK_LINES = 160;
export const LIGHTWEIGHT_PREVIEW_LINES = 240;

export function shouldUseLightweightCode(code: string): boolean {
  if (code.length > HIGHLIGHT_CHAR_LIMIT) return true;
  let lines = 1;
  for (let index = 0; index < code.length; index++) {
    if (code.charCodeAt(index) !== 10) continue;
    lines++;
    if (lines > HIGHLIGHT_LINE_LIMIT) return true;
  }
  return false;
}

/** Split a large block into independently layout-contained line groups. */
export function chunkCodeForRendering(code: string, linesPerChunk = LIGHTWEIGHT_CHUNK_LINES): string[] {
  const lines = code.split("\n");
  const chunks: string[] = [];
  for (let start = 0; start < lines.length; start += linesPerChunk) {
    chunks.push(lines.slice(start, start + linesPerChunk).join("\n"));
  }
  return chunks;
}

export function previewCodeForRendering(code: string, maxLines = LIGHTWEIGHT_PREVIEW_LINES): string {
  let end = 0;
  let lines = 1;
  while (end < code.length && lines <= maxLines) {
    const next = code.indexOf("\n", end);
    if (next < 0) return code;
    end = next + 1;
    lines++;
  }
  return code.slice(0, end).replace(/\n$/, "");
}
