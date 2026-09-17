// CodeMirror adapters for the shared Obsidian Markdown commands in
// markdownEditing.ts. The native touch textarea applies the same pure edits, so
// both composer engines produce identical documents for the same action.
import { insertNewline } from "@codemirror/commands";
import { indentUnit } from "@codemirror/language";
import { Prec } from "@codemirror/state";
import { type EditorView, keymap } from "@codemirror/view";
import {
  continueMarkdownList,
  type EditorTextSelection,
  indentLines,
  type InlineFormat,
  type MarkdownEdit,
  newlineAndIndentOnly,
  outdentLines,
  toggleBlockquote,
  toggleBulletList,
  toggleChecklist,
  toggleInlineFormat,
  toggleNumberedList,
} from "./markdownEditing";

export type MarkdownEditCommand = (
  doc: string,
  selection: EditorTextSelection,
) => MarkdownEdit | null;

/** Apply one shared edit as a single undoable CM6 transaction. */
export function runMarkdownEdit(
  view: EditorView,
  command: MarkdownEditCommand,
  userEvent = "input.type",
): boolean {
  if (view.state.readOnly) return false;
  const { anchor, head } = view.state.selection.main;
  const edit = command(view.state.doc.toString(), { anchor, head });
  if (!edit) return false;
  view.dispatch({
    changes: edit.changes,
    selection: edit.selection,
    scrollIntoView: true,
    userEvent,
  });
  return true;
}

function indentUnitText(view: EditorView): string {
  return view.state.facet(indentUnit);
}

// Obsidian's editor keymap (smartIndentList on): Enter continues a list or
// quote, falls back to plain newline-and-indent; Shift-Enter keeps the list
// indentation without a new marker; Tab/Shift-Tab indent quote-aware and always
// keep focus in the editor.
export const obsidianMarkdownKeymap = Prec.high(keymap.of([
  {
    key: "Enter",
    run: (view) => runMarkdownEdit(view, continueMarkdownList),
    shift: (view) =>
      runMarkdownEdit(view, newlineAndIndentOnly) || insertNewline(view),
  },
  {
    key: "Tab",
    run: (view) => {
      runMarkdownEdit(
        view,
        (doc, selection) => indentLines(doc, selection, indentUnitText(view)),
        "input.indent",
      );
      return true;
    },
    shift: (view) => {
      runMarkdownEdit(
        view,
        (doc, selection) =>
          outdentLines(doc, selection, indentUnitText(view), view.state.tabSize),
        "delete.dedent",
      );
      return true;
    },
  },
]));

const INLINE_FORMAT_BY_MARKER: Record<string, InlineFormat> = {
  "**": "bold",
  "*": "italic",
  "`": "code",
  "==": "highlight",
  "~~": "strikethrough",
  "$": "math",
  "%%": "comment",
};

/** The toolbar's marker vocabulary mapped to Obsidian's formatting commands. */
export function inlineFormatCommand(marker: string): MarkdownEditCommand | null {
  const format = INLINE_FORMAT_BY_MARKER[marker];
  return format === undefined
    ? null
    : (doc, selection) => toggleInlineFormat(doc, selection, format);
}

/** The toolbar's line-prefix vocabulary mapped to Obsidian's list commands. */
export function linePrefixCommand(prefix: string): MarkdownEditCommand | null {
  switch (prefix) {
    case "- ":
      return toggleBulletList;
    case "1. ":
      return toggleNumberedList;
    case "- [ ] ":
      return toggleChecklist;
    case "> ":
      return toggleBlockquote;
    default:
      return null;
  }
}

/** Obsidian's indent unit for the native textarea, which has no CM6 facet. */
export const NATIVE_INDENT_UNIT = "\t";
