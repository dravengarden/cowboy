// A worktree revision covers every file in the session. The open document only
// changes when its own revision does, and a reader must never lose their place
// because the agent touched some other file.

export type DocumentRefreshDecision = "ignore" | "apply" | "prompt";

/** How long after the reader returns to Review a detected change still counts
 *  as something they have not read yet, so it applies without a prompt. */
export const DOCUMENT_REFRESH_RESUME_GRACE_MS = 2_500;

export function documentRefreshDecision({
  currentRevision,
  nextRevision,
  currentText,
  nextText,
  reading,
  now,
  autoApplyUntil,
}: {
  currentRevision: string | undefined;
  nextRevision: string | undefined;
  currentText: string;
  nextText: string;
  reading: boolean;
  now: number;
  autoApplyUntil: number;
}): DocumentRefreshDecision {
  // Some diff results carry no revision; their text is the identity then.
  const unchanged = currentRevision !== undefined && nextRevision !== undefined
    ? nextRevision === currentRevision
    : nextText === currentText;
  if (unchanged) return "ignore";
  return !reading || now < autoApplyUntil ? "apply" : "prompt";
}

/** The first rendered block at the top of the viewport, identified by its text
 *  so a refresh that inserts or removes content above it cannot move it. */
export type TextScrollAnchor = {
  text: string;
  occurrence: number;
  offset: number;
};

export function normalizedAnchorText(value: string | null): string {
  return (value ?? "").replace(/\s+/g, " ").trim().slice(0, 240);
}

/** Index of the block matching `anchor` in a refreshed document. Repeated
 *  identical blocks keep their ordinal; when fewer remain, the last one wins. */
export function textAnchorIndex(
  texts: readonly string[],
  anchor: Pick<TextScrollAnchor, "text" | "occurrence">,
): number | undefined {
  let last: number | undefined;
  let seen = 0;
  for (const [index, text] of texts.entries()) {
    if (text !== anchor.text) continue;
    if (seen === anchor.occurrence) return index;
    seen += 1;
    last = index;
  }
  return last;
}

export type LineScrollAnchor = { line: number; text: string; offset: number };

const LINE_ANCHOR_SEARCH_RADIUS = 400;

/** 1-based line that should sit where `anchor` was: the nearest line with the
 *  same content, else the same line number clamped to the new document. */
export function lineAnchorTarget(
  lineCount: number,
  lineText: (line: number) => string,
  anchor: Pick<LineScrollAnchor, "line" | "text">,
): number {
  const clamped = Math.max(1, Math.min(anchor.line, lineCount));
  if (anchor.text.trim() === "") return clamped;
  for (let distance = 0; distance <= LINE_ANCHOR_SEARCH_RADIUS; distance += 1) {
    for (const line of [clamped + distance, clamped - distance]) {
      if (line >= 1 && line <= lineCount && lineText(line) === anchor.text) {
        return line;
      }
    }
  }
  return clamped;
}

const ANCHOR_BLOCKS =
  "h1, h2, h3, h4, h5, h6, p, li, pre, blockquote, tr, .cm-line";

type AnchorScroller = Pick<
  HTMLElement,
  "getBoundingClientRect" | "querySelectorAll" | "scrollTop"
>;

export function captureTextScrollAnchor(
  scroller: AnchorScroller,
): TextScrollAnchor | undefined {
  const top = scroller.getBoundingClientRect().top;
  const blocks = Array.from(
    scroller.querySelectorAll<HTMLElement>(ANCHOR_BLOCKS),
  );
  const counts = new Map<string, number>();
  for (const block of blocks) {
    const text = normalizedAnchorText(block.textContent);
    if (!text) continue;
    const occurrence = counts.get(text) ?? 0;
    counts.set(text, occurrence + 1);
    const rect = block.getBoundingClientRect();
    if (rect.bottom > top + 1) {
      return { text, occurrence, offset: rect.top - top };
    }
  }
  return undefined;
}

/** Scroll so the anchored block is back at its captured offset. Returns false
 *  when the block no longer exists; the caller keeps the numeric position. */
export function restoreTextScrollAnchor(
  scroller: AnchorScroller,
  anchor: TextScrollAnchor,
): boolean {
  const blocks = Array.from(
    scroller.querySelectorAll<HTMLElement>(ANCHOR_BLOCKS),
  );
  const index = textAnchorIndex(
    blocks.map((block) => normalizedAnchorText(block.textContent)),
    anchor,
  );
  const block = index === undefined ? undefined : blocks[index];
  if (!block) return false;
  const top = scroller.getBoundingClientRect().top;
  scroller.scrollTop += block.getBoundingClientRect().top - top - anchor.offset;
  return true;
}
