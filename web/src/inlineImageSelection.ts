interface InlineImageInsertionAttachment {
  id: string;
  name: string;
}

export interface InlineImageInsertion {
  from: number;
  to: number;
  insert: string;
  caret: number;
}

/** Build the one document replacement shared by native textarea and CM6 paste. */
export function inlineImageInsertion(
  value: string,
  anchor: number,
  head: number,
  attachments: readonly InlineImageInsertionAttachment[],
): InlineImageInsertion {
  const clampedAnchor = Math.max(0, Math.min(anchor, value.length));
  const clampedHead = Math.max(0, Math.min(head, value.length));
  const from = Math.min(clampedAnchor, clampedHead);
  const to = Math.max(clampedAnchor, clampedHead);
  const lineStart = value.lastIndexOf("\n", from - 1) + 1;
  const insert = attachments.map((attachment, index) => {
    const lead = index === 0 && from !== lineStart ? "\n" : "";
    const label = attachment.name.replaceAll("]", "");
    return `${lead}![${label}](cowboy-att:${attachment.id})\n`;
  }).join("");
  return { from, to, insert, caret: from + insert.length };
}

/** Map a CodeMirror position through removal of one image block. */
export function mapImageDeletionPosition(
  position: number,
  from: number,
  to: number,
): number {
  if (position <= from) return position;
  if (position >= to) return position - (to - from);
  return from;
}

/** Remove the image token and the surrounding line breaks used by insertion. */
export function imageDeletionRange(
  lineFrom: number,
  lineTo: number,
  docLength: number,
): { from: number; to: number } {
  return {
    from: lineFrom > 0 ? lineFrom - 1 : lineFrom,
    to: Math.min(docLength, lineTo + 1),
  };
}
