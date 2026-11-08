import { EditorView } from "@codemirror/view";

export interface ReviewEditorSelection {
  /** One-based document lines, inclusive. */
  readonly from: number;
  readonly to: number;
}


/** Selected lines: Visual mode's selection, else the cursor line. */
export function editorSelection(view: EditorView): ReviewEditorSelection {
  const { from, to, empty } = view.state.selection.main;
  const doc = view.state.doc;
  const first = doc.lineAt(from).number;
  // A linewise Visual selection ends at the start of the next line.
  const end = !empty && to > from && doc.lineAt(to).from === to ? to - 1 : to;
  return { from: first, to: Math.max(first, doc.lineAt(end).number) };
}

/** Move the cursor to `line` and centre it. */
export function revealEditorLine(view: EditorView, line: number): void {
  const target = view.state.doc.line(Math.min(Math.max(1, line), view.state.doc.lines));
  view.dispatch({
    selection: { anchor: target.from },
    effects: EditorView.scrollIntoView(target.from, { y: "center" }),
  });
}
