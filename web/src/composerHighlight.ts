// `==text==` → a `Highlight` node, Obsidian-style. lezer's GFM markdown has no
// highlight rule, so cowboy adds one as a `@lezer/markdown` inline extension —
// a faithful mirror of the built-in Strikethrough (`~~`): a doubled delimiter
// (`==`) with the same CommonMark flanking rules so whitespace-edged `== x ==`
// doesn't open/close. mdlive renders the result via its generic mark decorations
// (see composerExtensions.ts wiring + the `Highlight`/`HighlightMark` entries in
// mdlive/inline-preview.ts), so the marker hides on inactive lines and reveals
// raw on the active line exactly like strike/bold — and it's a `mark` decoration,
// never a contenteditable widget, so it's IME-safe.
import type { InlineContext, MarkdownConfig } from "@lezer/markdown";

const EQ = 61; // "="

// ASCII CommonMark punctuation — drives the flanking rules so `==` only
// opens/closes against non-space edges. (lezer's emphasis code also folds in the
// U+2010–2027 unicode dashes; immaterial for `==highlight==`, so ASCII suffices.)
const PUNCT = /[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~]/;

const HighlightDelim = { resolve: "Highlight", mark: "HighlightMark" };

export const Highlight: MarkdownConfig = {
  defineNodes: [{ name: "Highlight" }, { name: "HighlightMark" }],
  parseInline: [
    {
      name: "Highlight",
      parse(cx: InlineContext, next: number, pos: number): number {
        // Exactly `==` — not a lone `=` and not `===…` (an HR/setext-ish run we
        // leave alone).
        if (next !== EQ || cx.char(pos + 1) !== EQ || cx.char(pos + 2) === EQ) {
          return -1;
        }
        const before = cx.slice(pos - 1, pos);
        const after = cx.slice(pos + 2, pos + 3);
        const spaceBefore = /\s|^$/.test(before);
        const spaceAfter = /\s|^$/.test(after);
        const punctBefore = PUNCT.test(before);
        const punctAfter = PUNCT.test(after);
        return cx.addDelimiter(
          HighlightDelim,
          pos,
          pos + 2,
          !spaceAfter && (!punctAfter || spaceBefore || punctBefore),
          !spaceBefore && (!punctBefore || spaceAfter || punctAfter),
        );
      },
      after: "Emphasis",
    },
  ],
};
