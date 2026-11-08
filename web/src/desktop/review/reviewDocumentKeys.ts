// Review's additions to the Document's Vim buffer (docs/desktop-code-review.md
// "Document: a read-only Vim buffer"). They are resolved per editor from
// codemirror-vim's pending input rather than registered in its global keymap,
// so the Composer and Draft editors mounted beside Review keep plain Vim, and
// a user mapping of the same keys wins.

import type { VimMapping } from "../vim/vimConfig";

export type ReviewDocumentAction =
  | { readonly kind: "hunk"; readonly offset: 1 | -1 }
  | { readonly kind: "file"; readonly offset: 1 | -1 }
  | { readonly kind: "unreviewed"; readonly offset: 1 | -1 }
  | { readonly kind: "tab"; readonly offset: number }
  | { readonly kind: "nthTab"; readonly n: number }
  | { readonly kind: "send" };

export interface ReviewVimInput {
  /** codemirror-vim's pending multi-key prefix (`]`, `g`). */
  readonly keyBuffer: string;
  readonly operator: boolean;
  /** The typed count (`3` in `3gt`), 0 without one. */
  readonly count: number;
  readonly visual: boolean;
}

const BRACKETS: Readonly<Record<string, ReviewDocumentAction["kind"]>> = {
  c: "hunk",
  f: "file",
  u: "unreviewed",
};

/** The Review action for the next Vim token, or null to leave it to Vim. */
export function reviewDocumentAction(
  input: ReviewVimInput,
  key: string,
  userMapped: (sequence: string, visual: boolean) => boolean,
): ReviewDocumentAction | null {
  if (input.operator) return null;
  const sequence = input.keyBuffer + key;
  if (userMapped(sequence, input.visual)) return null;
  if (sequence === "<CR>") return { kind: "send" };
  if (input.visual) return null;
  if ((input.keyBuffer === "]" || input.keyBuffer === "[") && BRACKETS[key]) {
    const kind = BRACKETS[key] as "hunk" | "file" | "unreviewed";
    return { kind, offset: input.keyBuffer === "]" ? 1 : -1 };
  }
  if (sequence === "gt") {
    return input.count > 0 ? { kind: "nthTab", n: input.count } : { kind: "tab", offset: 1 };
  }
  if (sequence === "gT") return { kind: "tab", offset: -Math.max(1, input.count) };
  return null;
}

/** Whether the user's vimrc maps exactly this sequence in this mode. */
export function userMapsSequence(
  mappings: readonly VimMapping[],
  sequence: string,
  visual: boolean,
): boolean {
  const mode = visual ? "visual" : "normal";
  return mappings.some((mapping) =>
    mapping.lhs === sequence && (mapping.mode === undefined || mapping.mode === mode)
  );
}

/** The next anchor line after (or before) `line` in sorted `lines`. */
export function adjacentLine(
  lines: readonly number[],
  line: number,
  offset: 1 | -1,
): number | undefined {
  return offset > 0
    ? lines.find((candidate) => candidate > line)
    : lines.findLast((candidate) => candidate < line);
}

/** The queue entry `offset` from `index`; `-1` (not in the queue) starts at
 *  the first or last entry. Stops at the ends. */
export function adjacentIndex(
  length: number,
  index: number,
  offset: 1 | -1,
  accept: (index: number) => boolean = () => true,
): number | undefined {
  let at = index < 0 ? (offset > 0 ? -1 : length) : index;
  for (;;) {
    at += offset;
    if (at < 0 || at >= length) return undefined;
    if (accept(at)) return at;
  }
}
