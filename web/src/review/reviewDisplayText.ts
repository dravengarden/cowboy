/** Render and capture the same LF text, preserving BOMs and Unicode identity. */
export function reviewDisplayText(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}
