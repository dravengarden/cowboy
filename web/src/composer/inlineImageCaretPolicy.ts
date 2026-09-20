import { type EditorState } from "@codemirror/state";
import { type EditorView } from "@codemirror/view";
import { isImageOnlyLine } from "../inlineImageSelection";

/** An image ROW: a line that carries nothing but inline image tokens. Pasting
 * two pictures fills one row, so this must not require a single token. */
export function isLoneImageTokenLine(text: string): boolean {
  return isImageOnlyLine(text);
}

/** Positions of every empty line whose previous line is a block image. */
export function emptyLinePositionsAfterImages(state: EditorState): number[] {
  const positions: number[] = [];
  const { doc } = state;
  for (let i = 1; i < doc.lines; i++) {
    if (!isLoneImageTokenLine(doc.line(i).text)) continue;
    const next = doc.line(i + 1);
    if (next.length === 0) positions.push(next.from);
  }
  return positions;
}

/** True when a collapsed caret is on an empty line immediately after an image. */
export function selectionOnEmptyLineAfterImage(state: EditorState): boolean {
  const selection = state.selection.main;
  if (!selection.empty) return false;
  const line = state.doc.lineAt(selection.head);
  return line.length === 0 &&
    line.number > 1 &&
    isLoneImageTokenLine(state.doc.line(line.number - 1).text);
}

export function documentHasLoneImageLine(state: EditorState): boolean {
  for (let i = 1; i <= state.doc.lines; i++) {
    if (isLoneImageTokenLine(state.doc.line(i).text)) return true;
  }
  return false;
}

/**
 * Empty lines that sit in the chain under a block image: the landing line
 * itself, or a later empty line whose previous line is also empty.
 */
export function selectionOnEmptyLineInImageChain(state: EditorState): boolean {
  const selection = state.selection.main;
  if (!selection.empty) return false;
  const line = state.doc.lineAt(selection.head);
  if (line.length > 0 || !documentHasLoneImageLine(state)) return false;
  if (line.number <= 1) return false;
  const previous = state.doc.line(line.number - 1);
  return previous.length === 0 || isLoneImageTokenLine(previous.text);
}

/** True when a collapsed caret is on a lone image token line. */
export function selectionOnLoneImageLine(state: EditorState): boolean {
  const selection = state.selection.main;
  return selection.empty &&
    isLoneImageTokenLine(state.doc.lineAt(selection.head).text);
}

export function caretOffImageLineSpec(_state: EditorState): {
  changes?: { from: number; insert: string };
  selection: { anchor: number };
} | null {
  // The image token is a real `.cm-line`. Return must stay a normal
  // CM6 / iOS line break on that line, not a jump onto a fake landing line.
  return null;
}

/** Kept so ComposerEditor's Enter binding stays stable. Always a no-op. */
export function moveCaretOffImageLine(_view: EditorView): boolean {
  return false;
}

/**
 * True when a collapsed caret sits on the whitespace-only landing line that
 * image insertion leaves under a lone image token (`token\n `). Obsidian's
 * list Enter dedents an indentation-only line instead of breaking it; that
 * landing space exists for iOS caret geometry, so Return must still break
 * the line there.
 */
export function selectionOnImageLandingLine(state: EditorState): boolean {
  const selection = state.selection.main;
  if (!selection.empty) return false;
  const line = state.doc.lineAt(selection.head);
  return line.number > 1 && line.text.trim() === "" &&
    isLoneImageTokenLine(state.doc.line(line.number - 1).text);
}
