// Engine-neutral markdown editing commands shared by the CodeMirror 6 composer
// and the native iOS <textarea> composer.
//
// Every command is a pure function over a "\n"-separated document string and a
// single selection. It returns a `MarkdownEdit` (changes in ORIGINAL document
// coordinates plus the resulting selection) which each engine applies in its
// own way: CodeMirror dispatches the change specs, the textarea splices text.
//
// The behaviour is a port of Obsidian's editor commands (Obsidian 1.13 app.js):
// `processLines` and its list/quote/heading toggles, the smart-list Enter and
// Shift-Enter handlers, `insertBlock`, `insertMarkdownLink`,
// `toggleMarkdownFormatting` and the quote-aware `indentMore`/`indentLess`
// replacements. Obsidian's inline-format detection walks a CodeMirror 5 mode
// token tree; here the same semantics are re-expressed over a lezer markdown
// tree (bold/italic/code/highlight/strikethrough) or a same-line text scan
// (math/comment), which have no lezer nodes.
import { markdownLanguage } from "@codemirror/lang-markdown";
import type { MarkdownParser } from "@lezer/markdown";
import { Highlight } from "../composerHighlight";

export interface EditorTextSelection {
  anchor: number;
  head: number;
}

export interface TextChange {
  from: number;
  to: number;
  insert: string;
}

/** changes: sorted, non-overlapping, in ORIGINAL doc coordinates (like a CM6 ChangeSpec array). selection: in RESULTING doc coordinates. */
export interface MarkdownEdit {
  changes: TextChange[];
  selection: EditorTextSelection;
}

export type InlineFormat =
  | "bold"
  | "italic"
  | "code"
  | "highlight"
  | "strikethrough"
  | "math"
  | "comment";

interface TextRange {
  from: number;
  to: number;
}

// ---------------------------------------------------------------------------
// Document helpers
// ---------------------------------------------------------------------------

interface DocLine {
  /** 0-based line index. */
  index: number;
  from: number;
  to: number;
  text: string;
}

class DocLines {
  readonly lines: DocLine[] = [];

  constructor(readonly doc: string) {
    let from = 0;
    for (const text of doc.split("\n")) {
      this.lines.push({ index: this.lines.length, from, to: from + text.length, text });
      from += text.length + 1;
    }
  }

  get count(): number {
    return this.lines.length;
  }

  line(index: number): DocLine {
    const line = this.lines[index];
    if (!line) throw new RangeError(`Line ${index} out of range`);
    return line;
  }

  lineAt(pos: number): DocLine {
    const clamped = Math.max(0, Math.min(pos, this.doc.length));
    let low = 0;
    let high = this.lines.length - 1;
    while (low < high) {
      const mid = (low + high + 1) >> 1;
      if (this.line(mid).from <= clamped) low = mid;
      else high = mid - 1;
    }
    return this.line(low);
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/** Applies a `MarkdownEdit`'s changes (original coordinates) to `doc`. */
export function applyMarkdownEdit(doc: string, edit: MarkdownEdit): string {
  let result = "";
  let pos = 0;
  for (const change of edit.changes) {
    if (change.from > pos) result += doc.slice(pos, change.from);
    result += change.insert;
    pos = Math.max(pos, change.to);
  }
  return result + doc.slice(pos);
}

/**
 * Sorts changes (stable by `from`) and merges overlapping ones, mirroring how
 * CodeMirror's `ChangeSet.of` composes an unsorted ChangeSpec array: text
 * inserted by a later spec lands after text inserted by an earlier spec at the
 * same position. No-op changes are dropped.
 */
function normalizeChanges(changes: readonly TextChange[]): TextChange[] {
  const sorted = changes
    .map((change, order) => ({ change, order }))
    .filter(({ change }) => change.from !== change.to || change.insert !== "")
    .sort((a, b) => a.change.from - b.change.from || a.order - b.order)
    .map(({ change }) => ({ ...change }));
  const merged: TextChange[] = [];
  for (const change of sorted) {
    const previous = merged[merged.length - 1];
    if (previous && (change.from < previous.to || change.from === previous.from)) {
      previous.to = Math.max(previous.to, change.to);
      previous.insert += change.insert;
    } else {
      merged.push(change);
    }
  }
  return merged;
}

/**
 * Maps a position through sorted, non-overlapping changes exactly like
 * CodeMirror's `ChangeDesc.mapPos(pos, assoc)`:
 * - a pure insertion AT `pos` keeps the position before the inserted text for
 *   `assoc < 0` and pushes it past the text for `assoc > 0`;
 * - a position at the start of a replaced range stays at the replacement start;
 * - a position strictly inside a replaced range goes to the replacement start
 *   (`assoc < 0`) or end (`assoc > 0`);
 * - a position at the end of a replaced range lands after the replacement.
 */
function mapPos(changes: readonly TextChange[], pos: number, assoc: -1 | 1): number {
  let delta = 0;
  for (const change of changes) {
    if (change.from > pos) break;
    const length = change.to - change.from;
    const inserted = change.insert.length;
    if (change.from === pos) {
      if (length > 0 || assoc < 0) return pos + delta;
      delta += inserted;
      continue;
    }
    if (change.to > pos) return change.from + delta + (assoc < 0 ? 0 : inserted);
    delta += inserted - length;
  }
  return pos + delta;
}

/**
 * CodeMirror's default selection mapping for a transaction without an explicit
 * selection (`SelectionRange.map(changes)`): a caret maps with assoc -1, a
 * range maps its start with assoc +1 and its end with assoc -1.
 */
function mapSelectionDefault(
  changes: readonly TextChange[],
  sel: EditorTextSelection,
): EditorTextSelection {
  if (sel.anchor === sel.head) {
    const pos = mapPos(changes, sel.head, -1);
    return { anchor: pos, head: pos };
  }
  const forward = sel.head >= sel.anchor;
  const from = mapPos(changes, Math.min(sel.anchor, sel.head), 1);
  const to = Math.max(from, mapPos(changes, Math.max(sel.anchor, sel.head), -1));
  return forward ? { anchor: from, head: to } : { anchor: to, head: from };
}

function makeEdit(changes: readonly TextChange[], selection: EditorTextSelection): MarkdownEdit {
  return { changes: normalizeChanges(changes), selection };
}

// ---------------------------------------------------------------------------
// Line prefix grammar (Obsidian `gI` / `yI` / `zP`)
// ---------------------------------------------------------------------------

/** Quote/indent prefix, then an optional list marker with an optional task box. */
const LIST_PREFIX = /^([>\s]*)(([*+-] |(\d+)([.)] ))(?:\[(.)\] )?)?/;
/** Quote/indent prefix, an optional ATX heading marker, then the text. */
const HEADING_PREFIX = /^([>\s]*)(#{1,6} )?(.*)/;
const BLOCKQUOTE_PREFIX = /^\s{0,3}>(\s*)/;

interface ListPrefix {
  /** Whole matched prefix (group 0); may be empty. */
  whole: string;
  /** Leading quote/whitespace run (group 1). */
  lead: string;
  /** Marker including any task box (group 2), "" when absent. */
  marker: string;
  /** Bullet or number marker with its delimiter and space (group 3). */
  bullet: string;
  /** Ordered-list number digits (group 4). */
  number: string;
  /** Ordered-list delimiter plus space (group 5). */
  delimiter: string;
  /** Task box character (group 6), "" when absent. */
  box: string;
}

function matchListPrefix(text: string): ListPrefix {
  const match = LIST_PREFIX.exec(text);
  return {
    whole: match?.[0] ?? "",
    lead: match?.[1] ?? "",
    marker: match?.[2] ?? "",
    bullet: match?.[3] ?? "",
    number: match?.[4] ?? "",
    delimiter: match?.[5] ?? "",
    box: match?.[6] ?? "",
  };
}

/** Obsidian `qP`: length of the blockquote prefix to remove, 0 when not quoted. */
function blockquotePrefixLength(text: string): number {
  const match = BLOCKQUOTE_PREFIX.exec(text);
  if (!match) return 0;
  const spacing = match[1] ?? "";
  const throughMarker = match[0].length - spacing.length;
  let rest = spacing;
  while (rest.startsWith("    ")) rest = rest.substring(4);
  return rest.startsWith(" ") ? throughMarker + 1 : throughMarker;
}

// ---------------------------------------------------------------------------
// Fenced code blocks (stand-in for Obsidian's `HyperMD-codeblock` line class)
// ---------------------------------------------------------------------------

const FENCE_OPEN = /^ {0,3}(`{3,}|~{3,})(.*)$/;
const FENCE_CLOSE = /^ {0,3}(`{3,}|~{3,})[ \t]*$/;

/** Marks every line that belongs to a fenced code block, fence lines included. */
function codeBlockLineFlags(lines: DocLines): boolean[] {
  const flags: boolean[] = [];
  let fence: { char: string; length: number } | null = null;
  for (const line of lines.lines) {
    if (fence) {
      flags.push(true);
      const close = FENCE_CLOSE.exec(line.text);
      const run = close?.[1];
      if (run && run[0] === fence.char && run.length >= fence.length) fence = null;
      continue;
    }
    const open = FENCE_OPEN.exec(line.text);
    const run = open?.[1];
    const info = open?.[2] ?? "";
    if (run && !(run[0] === "`" && info.includes("`"))) {
      fence = { char: run[0] ?? "`", length: run.length };
      flags.push(true);
    } else {
      flags.push(false);
    }
  }
  return flags;
}

// ---------------------------------------------------------------------------
// processLines and the line toggles
// ---------------------------------------------------------------------------

/** A per-line change expressed in line-relative columns, like Obsidian's `{from, to?, text}`. */
interface LineChange {
  fromCh: number;
  toCh?: number;
  text: string;
}

/**
 * Obsidian `processLines`: analyses every line touched by the selection, then
 * builds at most one change per line. With `skipBlankLines` (the default) blank
 * lines of a multi-line selection are neither analysed nor changed.
 *
 * Selection: a collapsed caret at column 0 of the first changed line shifts by
 * that change's length delta (clamped to the line start); otherwise the
 * selection follows CodeMirror's default transaction mapping.
 */
function processLines<T>(
  doc: string,
  sel: EditorTextSelection,
  analyze: (text: string) => T,
  build: (text: string, analysis: T | null) => LineChange | null | undefined,
  skipBlankLines = true,
): MarkdownEdit | null {
  const lines = new DocLines(doc);
  const first = lines.lineAt(Math.min(sel.anchor, sel.head)).index;
  const last = lines.lineAt(Math.max(sel.anchor, sel.head)).index;
  const multiLine = last > first;
  const analyses: (T | null)[] = [];
  for (let index = first; index <= last; index++) {
    const text = lines.line(index).text;
    analyses.push(skipBlankLines && multiLine && !text.trim() ? null : analyze(text));
  }
  const changes: TextChange[] = [];
  let firstChange: { line: number; delta: number } | null = null;
  for (let index = first; index <= last; index++) {
    const analysis = analyses[index - first] ?? null;
    if (skipBlankLines && analysis === null) continue;
    const line = lines.line(index);
    const change = build(line.text, analysis);
    if (!change) continue;
    const toCh = change.toCh ?? change.fromCh;
    changes.push({ from: line.from + change.fromCh, to: line.from + toCh, insert: change.text });
    firstChange ??= { line: index, delta: change.text.length - (toCh - change.fromCh) };
  }
  if (!firstChange) return null;
  const caretLine = lines.lineAt(sel.head);
  if (sel.anchor === sel.head && sel.head === caretLine.from && caretLine.index === firstChange.line) {
    const edit = makeEdit(changes, sel);
    const resultLine = new DocLines(applyMarkdownEdit(doc, edit)).line(firstChange.line);
    const pos = resultLine.from + Math.max(0, firstChange.delta);
    edit.selection = { anchor: pos, head: pos };
    return edit;
  }
  const normalized = normalizeChanges(changes);
  return { changes: normalized, selection: mapSelectionDefault(normalized, sel) };
}

/** Obsidian `toggleBulletList`. Numbered or task lines count as "missing" a bullet. */
export function toggleBulletList(doc: string, sel: EditorTextSelection): MarkdownEdit | null {
  let add = false;
  return processLines(
    doc,
    sel,
    (text) => {
      const prefix = matchListPrefix(text);
      if (!prefix.bullet || prefix.number || prefix.box) add = true;
      return prefix;
    },
    (_text, prefix) =>
      prefix
        ? { fromCh: 0, toCh: prefix.whole.length, text: prefix.lead + (add ? "- " : "") }
        : null,
  );
}

/** Obsidian `toggleNumberList`: always renumbers to `1. ` when adding. */
export function toggleNumberedList(doc: string, sel: EditorTextSelection): MarkdownEdit | null {
  let add = false;
  return processLines(
    doc,
    sel,
    (text) => {
      const prefix = matchListPrefix(text);
      if (!prefix.number) add = true;
      return prefix;
    },
    (_text, prefix) =>
      prefix
        ? { fromCh: 0, toCh: prefix.whole.length, text: prefix.lead + (add ? "1. " : "") }
        : null,
  );
}

/**
 * Obsidian `toggleCheckList()` without an argument. The mode is decided over
 * all lines: any line without a box → add `[ ] ` (Obsidian quirk: lines that
 * already had a box lose it in this mode); else any `[ ]` → check all; else
 * uncheck all. Lines without a list marker get `- `.
 */
export function toggleChecklist(doc: string, sel: EditorTextSelection): MarkdownEdit | null {
  let mode: 1 | 2 | 3 = 3;
  return processLines(
    doc,
    sel,
    (text) => {
      const prefix = matchListPrefix(text);
      if (mode > 2 && prefix.box === " ") mode = 2;
      if (mode > 1 && !prefix.box) mode = 1;
      return prefix;
    },
    (_text, prefix) => {
      if (!prefix) return null;
      const base = prefix.lead + (prefix.bullet || "- ");
      let text = base;
      if (mode === 1) {
        if (!prefix.box) text = base + "[ ] ";
      } else if (mode === 2) {
        text = base + "[x] ";
      } else {
        text = base + "[ ] ";
      }
      return { fromCh: 0, toCh: prefix.whole.length, text };
    },
  );
}

/** Obsidian `toggleBlockquote`: blank lines participate (they get `> ` too). */
export function toggleBlockquote(doc: string, sel: EditorTextSelection): MarkdownEdit | null {
  let add = false;
  return processLines(
    doc,
    sel,
    (text) => {
      const length = blockquotePrefixLength(text);
      if (length === 0) add = true;
      return length;
    },
    (_text, length) => {
      const prefixLength = length ?? 0;
      if (add) return prefixLength === 0 ? { fromCh: 0, text: "> " } : undefined;
      return { fromCh: 0, toCh: prefixLength, text: "" };
    },
    false,
  );
}

/** Obsidian `setHeading(level)`; level 0 (or less) removes, levels clamp to 6. */
export function setHeading(
  doc: string,
  sel: EditorTextSelection,
  level: number,
): MarkdownEdit | null {
  const normalized = level <= 0 ? 0 : clamp(Math.floor(level), 1, 6);
  const marker = normalized === 0 ? "" : "#".repeat(normalized) + " ";
  return processLines(
    doc,
    sel,
    (text) => HEADING_PREFIX.exec(text),
    (text, match) => {
      if (!match) return null;
      const lead = match[1] ?? "";
      const rest = match[3] ?? "";
      return { fromCh: lead.length, toCh: text.length - rest.length, text: marker };
    },
  );
}

function headingLevel(text: string): number {
  const marker = HEADING_PREFIX.exec(text)?.[2];
  return marker ? marker.length - 1 : 0;
}

/** Cowboy extra: cycles the first selected line none → H1 → H2 → H3 → none for all lines. */
export function cycleHeading(doc: string, sel: EditorTextSelection): MarkdownEdit | null {
  const lines = new DocLines(doc);
  const current = headingLevel(lines.lineAt(Math.min(sel.anchor, sel.head)).text);
  const next = current >= 1 && current < 3 ? current + 1 : current === 0 ? 1 : 0;
  return setHeading(doc, sel, next);
}

// ---------------------------------------------------------------------------
// Enter / Shift-Enter
// ---------------------------------------------------------------------------

/**
 * Obsidian `newlineAndIndentContinueMarkdownList` (Enter with "smart lists").
 * Returns null where Obsidian falls back to CodeMirror's `newlineAndIndent`.
 * On a fenced-code line it delegates to `newlineAndIndentOnly` like Obsidian's
 * Enter keymap does.
 *
 * The inserted newline deliberately REPLACES the character before the caret
 * (`charAt(ch - 1) + "\n" + ...` from `ch - 1`), exactly as Obsidian does; the
 * change shape matters to mobile IME DOM diffing.
 */
export function continueMarkdownList(doc: string, sel: EditorTextSelection): MarkdownEdit | null {
  if (sel.anchor !== sel.head) return null;
  const lines = new DocLines(doc);
  const line = lines.lineAt(sel.head);
  if (codeBlockLineFlags(lines)[line.index]) return newlineAndIndentOnly(doc, sel);

  const text = line.text;
  const ch = sel.head - line.from;
  const current = matchListPrefix(text);
  if (!current.whole) return null;
  if (ch < current.whole.length || (!current.marker && !current.lead)) return null;

  // A continuation line (prefix only) inherits the list item above whose full
  // prefix has the same width.
  let item = current;
  if (!current.marker) {
    for (let index = line.index - 1; index >= 0; index--) {
      const above = matchListPrefix(lines.line(index).text);
      if (above.marker && above.whole.length === current.whole.length) {
        item = above;
        break;
      }
      if (above.whole.length < current.whole.length) break;
    }
  }
  const { lead, marker } = item;
  const changes: TextChange[] = [];
  const caret = sel.head;
  const at = (lineIndex: number, column: number) => lines.line(lineIndex).from + column;

  if (text.substring(item.whole.length).trim() === "") {
    // Empty item: remove one level of structure instead of continuing.
    if (!lead) {
      changes.push({ from: line.from, to: caret, insert: "" });
    } else if (lead.endsWith(">") || lead.endsWith("> ")) {
      if (marker) {
        changes.push({ from: at(line.index, lead.length), to: caret, insert: "" });
      } else {
        const quoteWidth = lead.endsWith(">") ? 1 : 2;
        const previous = line.index > 0 ? lines.line(line.index - 1) : null;
        if (previous && previous.text === lead) {
          changes.push(
            {
              from: at(previous.index, lead.length - quoteWidth),
              to: at(previous.index, lead.length),
              insert: "",
            },
            { from: at(line.index, lead.length - quoteWidth), to: at(line.index, lead.length), insert: "" },
          );
        } else {
          changes.push({
            from: at(line.index, lead.length - quoteWidth),
            to: at(line.index, lead.length),
            insert: "\n" + lead.substring(0, lead.length - quoteWidth),
          });
        }
      }
    } else if (lead.endsWith("\t")) {
      changes.push({ from: at(line.index, lead.length - 1), to: at(line.index, lead.length), insert: "" });
    } else {
      let spaces = 0;
      while (spaces < 4 && lead.charAt(lead.length - spaces - 1) === " ") spaces++;
      changes.push(
        spaces > 0
          ? { from: at(line.index, lead.length - spaces), to: at(line.index, lead.length), insert: "" }
          : { from: line.from, to: at(line.index, lead.length), insert: "" },
      );
    }
  } else if (!marker) {
    changes.push({ from: caret - 1, to: caret, insert: text.charAt(ch - 1) + "\n" + lead });
  } else {
    let nextMarker = item.bullet;
    if (item.number) nextMarker = String(parseInt(item.number, 10) + 1) + item.delimiter;
    if (item.box) nextMarker += "[ ] ";
    // Splitting before text that itself starts with a list marker reuses it.
    const after = matchListPrefix(text.substring(ch));
    if (after.bullet) nextMarker = "";
    changes.push({ from: caret - 1, to: caret, insert: text.charAt(ch - 1) + "\n" + lead + nextMarker });
    if (after.lead) changes.push({ from: caret, to: caret + after.lead.length, insert: "" });
  }
  return { changes, selection: mapSelectionDefault(changes, sel) };
}

/**
 * Obsidian `newlineAndIndentOnly` (Shift-Enter, and Enter inside fenced code):
 * keeps the quote/indent prefix and replaces the list marker with spaces of
 * equal width (tabs count as four). null → caller inserts a plain "\n".
 */
export function newlineAndIndentOnly(doc: string, sel: EditorTextSelection): MarkdownEdit | null {
  if (sel.anchor !== sel.head) return null;
  const line = new DocLines(doc).lineAt(sel.head);
  const ch = sel.head - line.from;
  const prefix = matchListPrefix(line.text);
  if (!prefix.whole || ch < prefix.whole.length) return null;
  const padding = prefix.marker.replace(/\t/g, "    ").replace(/./g, " ");
  const changes: TextChange[] = [
    { from: sel.head - 1, to: sel.head, insert: line.text.charAt(ch - 1) + "\n" + prefix.lead + padding },
  ];
  return { changes, selection: mapSelectionDefault(changes, sel) };
}

// ---------------------------------------------------------------------------
// Block insertions
// ---------------------------------------------------------------------------

/** Obsidian `insertBlock("```", "```")`: fences the touched lines; selection shifts by 4. */
export function insertCodeBlock(doc: string, sel: EditorTextSelection): MarkdownEdit {
  const lines = new DocLines(doc);
  const start = lines.lineAt(Math.min(sel.anchor, sel.head)).from;
  const end = lines.lineAt(Math.max(sel.anchor, sel.head)).to;
  const shift = "```".length + 1;
  return makeEdit(
    [
      { from: start, to: start, insert: "```\n" },
      { from: end, to: end, insert: "\n```" },
    ],
    { anchor: sel.anchor + shift, head: sel.head + shift },
  );
}

/** Obsidian `insertMarkdownLink`: `[selection]()`, caret in `()` or, when empty, in `[]`. */
export function insertMarkdownLink(doc: string, sel: EditorTextSelection): MarkdownEdit {
  const from = Math.min(sel.anchor, sel.head);
  const to = Math.max(sel.anchor, sel.head);
  const text = doc.slice(from, to);
  const caret = from + (from === to ? 1 : 1 + text.length + 2);
  return makeEdit([{ from, to, insert: "[" + text + "]()" }], { anchor: caret, head: caret });
}

// ---------------------------------------------------------------------------
// Quote-aware indentation (Obsidian `cZ` / `uZ` over `sZ`)
// ---------------------------------------------------------------------------

const INDENT_PREFIX = /^[\s>]+/;

/**
 * Obsidian `sZ`: runs `perLine` for every line the selection touches (a range
 * ending exactly at a line start excludes that line) and maps both selection
 * ends with assoc +1.
 */
function changeTouchedLines(
  doc: string,
  sel: EditorTextSelection,
  perLine: (line: DocLine, changes: TextChange[]) => void,
): MarkdownEdit | null {
  const lines = new DocLines(doc);
  const from = Math.min(sel.anchor, sel.head);
  const to = Math.max(sel.anchor, sel.head);
  const changes: TextChange[] = [];
  let lastLine = -1;
  for (let pos = from; pos <= to; ) {
    const line = lines.lineAt(pos);
    if (line.index > lastLine && (from === to || to > line.from)) {
      perLine(line, changes);
      lastLine = line.index;
    }
    pos = line.to + 1;
  }
  const normalized = normalizeChanges(changes);
  if (normalized.length === 0) return null;
  return {
    changes: normalized,
    selection: { anchor: mapPos(normalized, sel.anchor, 1), head: mapPos(normalized, sel.head, 1) },
  };
}

/** Obsidian `cZ` (indentMore): inserts `unit` after any `>` quote prefix. */
export function indentLines(doc: string, sel: EditorTextSelection, unit: string): MarkdownEdit | null {
  return changeTouchedLines(doc, sel, (line, changes) => {
    let insertAt = line.from;
    const prefix = INDENT_PREFIX.exec(line.text)?.[0];
    if (prefix) {
      let quote = prefix.lastIndexOf(">");
      if (quote !== -1) {
        if (prefix.charAt(quote + 1) === " ") quote++;
        insertAt = line.from + quote + 1;
      }
    }
    changes.push({ from: insertAt, to: insertAt, insert: unit });
  });
}

function countColumns(text: string, tabSize: number): number {
  let columns = 0;
  for (const char of text) columns = char === "\t" ? columns + tabSize - (columns % tabSize) : columns + 1;
  return columns;
}

/** CodeMirror `indentString`: tabs for whole tab stops when the unit is a tab. */
function indentString(columns: number, unit: string, tabSize: number): string {
  let result = "";
  let remaining = columns;
  let char = unit[0] ?? " ";
  if (char === "\t") {
    while (remaining >= tabSize) {
      result += "\t";
      remaining -= tabSize;
    }
    char = " ";
  }
  return result + char.repeat(Math.max(0, remaining));
}

/**
 * Obsidian `uZ` (indentLess): removes indentation before a leading `>`, or
 * shrinks the whitespace after the last `>` (or at line start) by one indent
 * unit measured in columns.
 */
export function outdentLines(
  doc: string,
  sel: EditorTextSelection,
  unit: string,
  tabSize = 4,
): MarkdownEdit | null {
  const unitColumns = unit.charCodeAt(0) === 9 ? tabSize * unit.length : unit.length;
  return changeTouchedLines(doc, sel, (line, changes) => {
    const prefix = INDENT_PREFIX.exec(line.text)?.[0];
    if (!prefix) return;
    const firstQuote = prefix.indexOf(">");
    if (firstQuote > 0) {
      changes.push({ from: line.from, to: line.from + firstQuote, insert: "" });
      return;
    }
    if (prefix === "> ") return;
    let lastQuote = prefix.lastIndexOf(">");
    if (lastQuote !== -1 && prefix.charAt(lastQuote + 1) === " ") lastQuote++;
    const whitespaceStart = lastQuote + 1;
    const whitespace = prefix.substring(whitespaceStart);
    if (!whitespace) return;
    const target = indentString(
      Math.max(0, countColumns(whitespace, tabSize) - unitColumns),
      unit,
      tabSize,
    );
    let common = 0;
    while (
      common < whitespace.length &&
      common < target.length &&
      whitespace.charCodeAt(common) === target.charCodeAt(common)
    ) {
      common++;
    }
    changes.push({
      from: line.from + whitespaceStart + common,
      to: line.from + prefix.length,
      insert: target.slice(common),
    });
  });
}

// ---------------------------------------------------------------------------
// Inline formatting (Obsidian `toggleMarkdownFormatting`)
// ---------------------------------------------------------------------------

interface InlineFormatSpec {
  marker: string;
  altMarker?: string;
  /** lezer node for the formatted span and its delimiter child; absent → text scan. */
  node?: string;
  mark?: string;
}

/** Obsidian `kp`. */
const INLINE_FORMATS: Record<InlineFormat, InlineFormatSpec> = {
  bold: { marker: "**", altMarker: "__", node: "StrongEmphasis", mark: "EmphasisMark" },
  italic: { marker: "*", altMarker: "_", node: "Emphasis", mark: "EmphasisMark" },
  code: { marker: "`", node: "InlineCode", mark: "CodeMark" },
  highlight: { marker: "==", node: "Highlight", mark: "HighlightMark" },
  strikethrough: { marker: "~~", node: "Strikethrough", mark: "StrikethroughMark" },
  math: { marker: "$" },
  comment: { marker: "%%" },
};

/** Inline delimiter nodes that Obsidian's CM5 mode styles as `formatting`. */
const FORMATTING_NODES = new Set([
  "EmphasisMark",
  "CodeMark",
  "StrikethroughMark",
  "HighlightMark",
  "LinkMark",
]);

let configuredParser: MarkdownParser | null = null;

function markdownParser(): MarkdownParser {
  configuredParser ??= (markdownLanguage.parser as MarkdownParser).configure([Highlight]);
  return configuredParser;
}

/** Everything toggleInlineFormat needs to know about one format in a document. */
interface FormatModel {
  /** Ranges of the format's spans (delimiters included). */
  spans: TextRange[];
  /** The format's own delimiter ranges. */
  marks: TextRange[];
  /** 1 where the character is any inline delimiter ("formatting" token). */
  formatting: Uint8Array;
  /** 1 where the character lies inside a span of the format. */
  inside: Uint8Array;
}

function buildFormatModel(doc: string, lines: DocLines, spec: InlineFormatSpec): FormatModel {
  const spans: TextRange[] = [];
  const marks: TextRange[] = [];
  const formatting = new Uint8Array(doc.length);
  const inside = new Uint8Array(doc.length);
  if (spec.node) {
    const tree = markdownParser().parse(doc);
    tree.iterate({
      enter(ref) {
        if (FORMATTING_NODES.has(ref.name)) formatting.fill(1, ref.from, ref.to);
        if (ref.name !== spec.node) return;
        spans.push({ from: ref.from, to: ref.to });
        for (let child = ref.node.firstChild; child; child = child.nextSibling) {
          if (child.name === spec.mark) marks.push({ from: child.from, to: child.to });
        }
      },
    });
  } else {
    for (const line of lines.lines) scanMarkerPairs(line, spec.marker, spans, marks);
    for (const mark of marks) formatting.fill(1, mark.from, mark.to);
  }
  for (const span of spans) inside.fill(1, span.from, span.to);
  marks.sort((a, b) => a.from - b.from);
  return { spans, marks, formatting, inside };
}

/**
 * Text fallback for `$` math and `%%` comments, which have no lezer nodes:
 * unescaped markers on one line pair up in order (nearest unmatched markers).
 * A `$$` run is display math and never an inline math delimiter.
 */
function scanMarkerPairs(line: DocLine, marker: string, spans: TextRange[], marks: TextRange[]): void {
  const found: number[] = [];
  const text = line.text;
  for (let i = 0; i < text.length; ) {
    if (text[i] === "\\") {
      i += 2;
      continue;
    }
    if (text.startsWith(marker, i)) {
      if (marker === "$" && text[i + 1] === "$") {
        while (text[i] === "$") i++;
        continue;
      }
      found.push(line.from + i);
      i += marker.length;
      continue;
    }
    i++;
  }
  for (let i = 0; i + 1 < found.length; i += 2) {
    const open = found[i] ?? 0;
    const close = found[i + 1] ?? 0;
    spans.push({ from: open, to: close + marker.length });
    marks.push({ from: open, to: open + marker.length }, { from: close, to: close + marker.length });
  }
}

const WHITESPACE = /\s/;

/** Obsidian `Cp`: trims whitespace from both ends of a range. */
function trimRange(doc: string, range: TextRange): TextRange {
  let from = range.from;
  let to = range.to;
  while (from < to && WHITESPACE.test(doc.charAt(from))) from++;
  while (to > from && WHITESPACE.test(doc.charAt(to - 1))) to--;
  return { from, to };
}

const HEADING_MARKER = /^#{1,6}(?:[ \t]+|$)/;

/**
 * Obsidian `xp`: the trimmed line content after its quote, list, task and
 * heading prefix.
 */
function lineContentRange(doc: string, line: DocLine): TextRange {
  const prefix = matchListPrefix(line.text);
  let start = prefix.whole.length;
  if (!prefix.marker) start += HEADING_MARKER.exec(line.text.substring(start))?.[0].length ?? 0;
  return trimRange(doc, { from: line.from + Math.min(start, line.text.length), to: line.to });
}

const WORD_CHAR = /[\p{Alphabetic}\p{Number}_]/u;

/** CodeMirror `EditorState.wordAt` with its default Unicode word categorizer. */
function wordAt(doc: string, line: DocLine, pos: number): TextRange | null {
  let start = pos;
  let end = pos;
  while (start > line.from) {
    const code = doc.codePointAt(start - 1) ?? 0;
    const width = start - 2 >= line.from && code >= 0xdc00 && code <= 0xdfff ? 2 : 1;
    if (!WORD_CHAR.test(doc.slice(start - width, start))) break;
    start -= width;
  }
  while (end < line.to) {
    const width = (doc.codePointAt(end) ?? 0) > 0xffff ? 2 : 1;
    if (!WORD_CHAR.test(doc.slice(end, end + width))) break;
    end += width;
  }
  return start === end ? null : { from: start, to: end };
}

/**
 * Obsidian `Mp` for one format: every non-delimiter character of the selection
 * (per line, limited to the line content, code-block lines skipped unless the
 * selection lies within that line) must be inside the format, and at least one
 * such character must exist. A caret probes the character before it (or the
 * one after it at a line start), like Obsidian's `moveTo(pos, -1)` token probe.
 */
function isFormatActive(
  doc: string,
  lines: DocLines,
  codeLines: readonly boolean[],
  model: FormatModel,
  range: TextRange,
): boolean {
  const firstLine = lines.lineAt(range.from);
  const lastLine = lines.lineAt(range.to);
  const multiLine = lastLine.index !== firstLine.index;
  let sawText = false;
  for (let index = firstLine.index; index <= lastLine.index; index++) {
    const line = lines.line(index);
    const withinLine = range.from >= line.from && range.to <= line.to;
    if (!withinLine && codeLines[index]) continue;
    const content = lineContentRange(doc, line);
    if (multiLine && content.from === content.to) continue;
    const start = Math.max(content.from, range.from);
    const end = Math.min(content.to, range.to);
    if (start < end) {
      for (let pos = start; pos < end; pos++) {
        if (model.formatting[pos]) continue;
        if (!model.inside[pos]) return false;
        sawText = true;
      }
      continue;
    }
    const probe = start > line.from ? start - 1 : start;
    if (probe >= line.to) return false;
    if (model.formatting[probe]) continue;
    if (!model.inside[probe]) return false;
    sawText = true;
  }
  return sawText;
}

/** Obsidian `Dp`: the marker (or alt marker) text directly after/before `pos`. */
function markerAt(doc: string, pos: number, spec: InlineFormatSpec, direction: 1 | -1 = 1): TextRange | null {
  for (const marker of spec.altMarker ? [spec.marker, spec.altMarker] : [spec.marker]) {
    const from = direction === -1 ? pos - marker.length : pos;
    const to = direction === -1 ? pos : pos + marker.length;
    if (from >= 0 && doc.slice(from, to) === marker) return { from, to };
  }
  return null;
}

/**
 * Obsidian `Tp`: deletions for every delimiter of the format touching `range`
 * (inclusive at both ends, like lezer's `iterate({from, to})`). Only the marker
 * width is removed, so a ``` `` ``` code delimiter loses one backtick.
 */
function removeFormatMarks(doc: string, model: FormatModel, range: TextRange, spec: InlineFormatSpec): TextChange[] {
  const changes: TextChange[] = [];
  for (const mark of model.marks) {
    if (mark.from > range.to || mark.to < range.from) continue;
    const marker = markerAt(doc, mark.from, spec);
    if (marker) changes.push({ ...marker, insert: "" });
  }
  return changes;
}

/**
 * Obsidian `Ap`: widens the trimmed selection to the outermost spans of the
 * format that contain its start and end.
 */
function enclosingFormatExtent(model: FormatModel, range: TextRange): TextRange {
  let from = range.from;
  let to = range.to;
  for (const span of model.spans) {
    if (span.from <= range.from && range.from < span.to) from = Math.min(from, span.from);
    if (span.from < range.to && range.to <= span.to) to = Math.max(to, span.to);
  }
  return { from, to };
}

/**
 * Obsidian's selection projection inside `toggleMarkdownFormatting`: shifts
 * `from` by changes ending at or before it and `to` by changes ending before
 * it (deletions ending at it included), then adds `offset`. The original
 * selection direction is preserved.
 */
function projectFormatSelection(
  sel: EditorTextSelection,
  from: number,
  to: number,
  changes: readonly TextChange[],
  offset = 0,
): EditorTextSelection {
  let fromShift = 0;
  let toShift = 0;
  for (const change of changes) {
    const delta = change.insert.length - (change.to - change.from);
    if (change.to <= from) fromShift += delta;
    if ((delta < 0 && change.to <= to) || change.to < to) toShift += delta;
  }
  const start = from + fromShift + offset;
  const end = from === to ? start : to + toShift + offset;
  return sel.head >= sel.anchor ? { anchor: start, head: end } : { anchor: end, head: start };
}

/**
 * Obsidian `toggleMarkdownFormatting` for a single selection range.
 *
 * - Active with a caret directly before the closing marker: step over it.
 * - Active otherwise: remove every delimiter of the enclosing span(s).
 * - Inactive: wrap the word at the caret (or an empty pair), or wrap each
 *   selected line's content separately, dropping inner delimiters of the same
 *   format first.
 */
export function toggleInlineFormat(
  doc: string,
  sel: EditorTextSelection,
  format: InlineFormat,
): MarkdownEdit | null {
  const spec = INLINE_FORMATS[format];
  const lines = new DocLines(doc);
  const codeLines = codeBlockLineFlags(lines);
  const model = buildFormatModel(doc, lines, spec);
  const range = { from: Math.min(sel.anchor, sel.head), to: Math.max(sel.anchor, sel.head) };
  const empty = range.from === range.to;
  const trimmed = trimRange(doc, range);
  const changes: TextChange[] = [];

  if (isFormatActive(doc, lines, codeLines, model, range)) {
    if (empty) {
      const closing = markerAt(doc, trimmed.from, spec);
      if (closing) {
        const width = closing.to - closing.from;
        return makeEdit([], projectFormatSelection(sel, trimmed.from, trimmed.from, [], width));
      }
    }
    const extent = enclosingFormatExtent(model, trimmed);
    changes.push(...removeFormatMarks(doc, model, extent, spec));
    const selection = projectFormatSelection(sel, trimmed.from, Math.min(trimmed.to, extent.to), changes);
    return makeEdit(changes, selection);
  }

  let start = trimmed.from;
  let end = trimmed.to;
  if (empty) {
    const word = wordAt(doc, lines.lineAt(start), start);
    if (word) {
      start = word.from;
      end = word.to;
    }
  }
  const firstLine = lines.lineAt(range.from).index;
  const lastLine = lines.lineAt(range.to).index;
  for (let index = firstLine; index <= lastLine; index++) {
    const line = lines.line(index);
    const withinLine = range.from >= line.from && range.to <= line.to;
    if (!withinLine && codeLines[index]) continue;
    let wrapFrom = start;
    let wrapTo = end;
    if (!empty) {
      const content = lineContentRange(doc, line);
      wrapFrom = clamp(content.from, start, end);
      wrapTo = clamp(content.to, start, end);
    }
    if (empty || wrapFrom !== wrapTo) {
      // Local deviation: an empty pair at a caret with no word must not also
      // delete a delimiter that merely touches the caret. Obsidian's literal
      // math turns `**foo**|` into the unbalanced `**foo**|**`.
      if (wrapFrom !== wrapTo) {
        changes.push(...removeFormatMarks(doc, model, { from: wrapFrom, to: wrapTo }, spec));
      }
      changes.push(
        { from: wrapFrom, to: wrapFrom, insert: spec.marker },
        { from: wrapTo, to: wrapTo, insert: spec.marker },
      );
    }
  }
  const selection = empty
    ? projectFormatSelection(sel, range.from, range.from, changes, start === end ? -spec.marker.length : 0)
    : projectFormatSelection(sel, start, end, changes);
  return makeEdit(changes, selection);
}
