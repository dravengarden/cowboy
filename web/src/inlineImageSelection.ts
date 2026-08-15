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

export function inlineImageTokenSpans(
  value: string,
): { from: number; to: number }[] {
  const tokens: { from: number; to: number }[] = [];
  const pattern = /!\[[^\]]*\]\(cowboy-att:[^)]+\)/g;
  for (const match of value.matchAll(pattern)) {
    tokens.push({ from: match.index, to: match.index + match[0].length });
  }
  return tokens;
}

/** A later Paste must not replace an already-placed thumbnail. iOS often
 * reports the first image's atomic range (or the whole document) as the
 * captured selection; substituting the pending token then deleting it on an
 * empty settle leaves the composer with no image. */
export function inlineImagePasteInsertion(
  value: string,
  anchor: number,
  head: number,
  attachments: readonly InlineImageInsertionAttachment[],
): InlineImageInsertion {
  const from = Math.min(anchor, head);
  const to = Math.max(anchor, head);
  const overlapped = inlineImageTokenSpans(value).filter((token) =>
    token.from < to && token.to > from
  );
  if (overlapped.length > 0) {
    const insertAt = Math.max(...overlapped.map((token) => token.to));
    return inlineImageInsertion(value, insertAt, insertAt, attachments);
  }
  return inlineImageInsertion(value, anchor, head, attachments);
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
  const restOnSameLine = to < value.length && value[to] !== "\n";
  const lead = from !== lineStart ? "\n" : "";
  const body = attachments.map((attachment) => {
    const label = attachment.name.replaceAll("]", "");
    return `![${label}](cowboy-att:${attachment.id})`;
  }).join("\n");
  // Physical v1265: a space on the image line made the caret as tall as
  // the thumbnail and Return still wrote <br> into that 88px line.
  // Keep the thumbnail on its own line; put a real space on the next
  // line so the caret is a normal 12px bar in a text node.
  const trail = restOnSameLine ? "\n" : "";
  const insert = `${lead}${body}\n ${trail}`;
  return { from, to, insert, caret: from + lead.length + body.length + 2 };
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
