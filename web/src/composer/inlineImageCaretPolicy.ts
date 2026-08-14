import { type EditorState } from "@codemirror/state";
import { type EditorView } from "@codemirror/view";

const LONE_IMAGE_TOKEN_RE = /^\s*!\[([^\]]*)\]\(cowboy-att:([^)]+)\)\s*$/;

export function isLoneImageTokenLine(text: string): boolean {
  return LONE_IMAGE_TOKEN_RE.test(text);
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
