// Pure Vim editing for native text fields (inputVim.ts). A field is one
// string; a textarea adds lines. Positions are UTF-16 indices, as the DOM
// selection API uses; the Normal cursor sits on a character, so it is at most
// `length - 1` (0 for an empty field).

export interface VimText {
  readonly value: string;
  readonly cursor: number;
}

type CharClass = 0 | 1 | 2; // space, word, punctuation

function charClass(char: string | undefined, big: boolean): CharClass {
  if (char === undefined || /\s/.test(char)) return 0;
  if (big) return 1;
  return /[\p{L}\p{N}_]/u.test(char) ? 1 : 2;
}

export function lineStart(value: string, pos: number): number {
  return value.lastIndexOf("\n", pos - 1) + 1;
}

export function lineEnd(value: string, pos: number): number {
  const end = value.indexOf("\n", pos);
  return end < 0 ? value.length : end;
}

/** The last character a Normal cursor may sit on in this line. */
export function normalClamp(value: string, pos: number): number {
  const start = lineStart(value, Math.min(pos, value.length));
  const end = lineEnd(value, start);
  return Math.max(start, Math.min(pos, end - 1));
}

export function firstNonBlank(value: string, pos: number): number {
  const start = lineStart(value, pos);
  const end = lineEnd(value, start);
  let index = start;
  while (index < end && /[ \t]/.test(value[index]!)) index++;
  return index;
}

/** `w`/`W`: start of the next word. */
export function wordForward(value: string, pos: number, big = false): number {
  let index = pos;
  const start = charClass(value[index], big);
  while (index < value.length && start !== 0 && charClass(value[index], big) === start) {
    index++;
  }
  while (index < value.length && charClass(value[index], big) === 0) index++;
  return index;
}

/** `b`/`B`: start of this or the previous word. */
export function wordBackward(value: string, pos: number, big = false): number {
  let index = pos - 1;
  while (index > 0 && charClass(value[index], big) === 0) index--;
  const cls = charClass(value[index], big);
  while (index > 0 && charClass(value[index - 1], big) === cls) index--;
  return Math.max(0, index);
}

/** `e`/`E`: end of this or the next word. */
export function wordEnd(value: string, pos: number, big = false): number {
  let index = pos + 1;
  while (index < value.length && charClass(value[index], big) === 0) index++;
  const cls = charClass(value[index], big);
  while (index + 1 < value.length && charClass(value[index + 1], big) === cls) {
    index++;
  }
  return Math.min(index, Math.max(0, value.length - 1));
}

/** `f`/`F`/`t`/`T` within the line; null when the character is absent. */
export function findChar(
  value: string,
  pos: number,
  kind: "f" | "F" | "t" | "T",
  char: string,
): number | null {
  const start = lineStart(value, pos);
  const end = lineEnd(value, pos);
  if (kind === "f" || kind === "t") {
    const found = value.indexOf(char, pos + (kind === "t" ? 2 : 1));
    if (found < 0 || found >= end) return null;
    return kind === "t" ? found - 1 : found;
  }
  const found = value.lastIndexOf(char, pos - (kind === "T" ? 2 : 1));
  if (found < start) return null;
  return kind === "T" ? found + 1 : found;
}

/** `j`/`k` in a textarea: the same column of the next/previous line. */
export function verticalMove(
  value: string,
  pos: number,
  delta: 1 | -1,
): number | null {
  const start = lineStart(value, pos);
  const column = pos - start;
  if (delta === 1) {
    const end = lineEnd(value, pos);
    if (end >= value.length) return null;
    const next = end + 1;
    return normalClamp(value, Math.min(next + column, lineEnd(value, next)));
  }
  if (start === 0) return null;
  const previous = lineStart(value, start - 1);
  return normalClamp(value, Math.min(previous + column, start - 1));
}

export interface VimRange {
  readonly from: number;
  readonly to: number;
  /** Whole lines (`dd`, `yy`): pasting puts them on their own line. */
  readonly linewise?: boolean;
}

/** The range an operator covers for a motion (Vim's inclusive rules). */
export function operatorRange(
  value: string,
  pos: number,
  motion: string,
  target: number,
  operator: "d" | "c" | "y",
): VimRange {
  // `cw` changes to the end of the word, like `ce`.
  if (operator === "c" && (motion === "w" || motion === "W")) {
    if (charClass(value[pos], motion === "W") !== 0) {
      return { from: pos, to: wordEnd(value, pos - 1, motion === "W") + 1 };
    }
  }
  const inclusive = ["e", "E", "f", "t", "$"].includes(motion);
  const from = Math.min(pos, target);
  let to = Math.max(pos, target) + (inclusive ? 1 : 0);
  // `dw` on the last word of a line stops at the line end.
  if (motion === "w" || motion === "W") to = Math.min(to, lineEnd(value, pos));
  return { from, to: Math.min(to, value.length) };
}

/** The current line, for `dd`/`cc`/`yy`. */
export function lineRange(value: string, pos: number): VimRange {
  const start = lineStart(value, pos);
  const end = lineEnd(value, pos);
  return { from: start, to: end, linewise: true };
}

export function removeRange(value: string, range: VimRange): string {
  if (range.linewise && value.includes("\n")) {
    // Remove the line and one of its separators.
    const to = range.to < value.length ? range.to + 1 : range.to;
    const from = range.to >= value.length && range.from > 0
      ? range.from - 1
      : range.from;
    return value.slice(0, from) + value.slice(to);
  }
  return value.slice(0, range.from) + value.slice(range.to);
}
