import { type EditorState } from "@codemirror/state";
import { type EditorView } from "@codemirror/view";

const LONE_IMAGE_TOKEN_RE = /^\s*!\[([^\]]*)\]\(cowboy-att:([^)]+)\)\s*$/;

export function isLoneImageTokenLine(text: string): boolean {
  return LONE_IMAGE_TOKEN_RE.test(text);
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

export function caretOffImageLineSpec(state: EditorState): {
  changes?: { from: number; insert: string };
  selection: { anchor: number };
} | null {
  if (!selectionOnLoneImageLine(state)) return null;
  const line = state.doc.lineAt(state.selection.main.head);
  if (line.number < state.doc.lines) {
    return { selection: { anchor: state.doc.line(line.number + 1).from } };
  }
  return {
    changes: { from: line.to, insert: "\n" },
    selection: { anchor: line.to + 1 },
  };
}

/**
 * If Return lands on the atomic image line, move onto the existing trailing
 * empty line instead of inserting a newline before the thumbnail.
 */
export function moveCaretOffImageLine(view: EditorView): boolean {
  const spec = caretOffImageLineSpec(view.state);
  if (!spec) return false;
  view.dispatch({ ...spec, scrollIntoView: true });
  return true;
}
