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

/** A line that holds nothing but inline image tokens. A thumbnail is capped at
 * 80px tall and roughly a third of the composer's width, so giving every image
 * its own line stacked an 88px row plus a 24px landing line per picture down
 * the left edge and turned a two-image composer into a ~290px mostly-blank
 * card. Consecutive images share one image row and flow across it instead. */
const IMAGE_ONLY_LINE_RE =
  /^\s*(?:!\[[^\]]*\]\(cowboy-att:[^)]+\)\s*)+$/;

export function isImageOnlyLine(text: string): boolean {
  return IMAGE_ONLY_LINE_RE.test(text);
}

/** The whitespace-only landing line image insertion leaves under an image row
 * so iOS has a normal-height text node to park the caret in. */
function isImageLandingLine(text: string): boolean {
  return /^ *$/.test(text);
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

/** Start of the line that ends at `lineEnd`. */
function lineStartBefore(value: string, lineEnd: number): number {
  return value.lastIndexOf("\n", lineEnd - 1) + 1;
}

/** End offset of the landing line directly under the row ending at `lineEnd`,
 * or null when that row has no landing line yet. */
function landingLineEndAfter(value: string, lineEnd: number): number | null {
  if (lineEnd >= value.length) return null;
  const nextStart = lineEnd + 1;
  const nextBreak = value.indexOf("\n", nextStart);
  const nextEnd = nextBreak === -1 ? value.length : nextBreak;
  return isImageLandingLine(value.slice(nextStart, nextEnd)) ? nextEnd : null;
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
  const lineBreak = value.indexOf("\n", to);
  const lineEnd = lineBreak === -1 ? value.length : lineBreak;
  const restOnSameLine = to < value.length && value[to] !== "\n";
  const lead = from !== lineStart ? "\n" : "";
  // One image row, not one row per picture: the tokens sit side by side and the
  // row wraps like text once it runs out of width.
  const body = attachments.map((attachment) => {
    const label = attachment.name.replaceAll("]", "");
    return `![${label}](cowboy-att:${attachment.id})`;
  }).join("");

  // Physical v1265: a space on the image line made the caret as tall as
  // the thumbnail and Return still wrote <br> into that 88px line. Keep the
  // caret off the image row; put a real space on the next line so it is a
  // normal 12px bar in a text node. That landing line is per ROW, so an image
  // arriving next to an existing row must reuse it rather than mint another.
  if (attachments.length > 0 && from === to) {
    const currentLine = value.slice(lineStart, lineEnd);
    if (isImageOnlyLine(currentLine)) {
      const landing = landingLineEndAfter(value, lineEnd);
      return landing === null
        ? {
          from: lineEnd,
          to: lineEnd,
          insert: `${body}\n `,
          caret: lineEnd + body.length + 2,
        }
        : {
          from: lineEnd,
          to: lineEnd,
          insert: body,
          caret: landing + body.length,
        };
    }
    const rowEnd = lineStart - 1;
    if (
      isImageLandingLine(currentLine) && rowEnd > 0 &&
      isImageOnlyLine(value.slice(lineStartBefore(value, rowEnd), rowEnd))
    ) {
      return {
        from: rowEnd,
        to: rowEnd,
        insert: body,
        caret: lineEnd + body.length,
      };
    }
  }

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
