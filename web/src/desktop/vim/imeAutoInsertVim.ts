import { Prec, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { getCM, Vim, vim } from "@replit/codemirror-vim";

/**
 * Desktop-only Vim with composition-aware mode ownership.
 *
 * Keep CodeMirror's contenteditable node stable at all times. When the OS starts
 * a real IME composition outside Insert/Replace, treat that as typing intent and
 * enter Insert before codemirror-vim's own normal-mode input handler can force-end
 * the composition by detaching contenteditable.
 */
export function createImeAutoInsertVim(): {
  extension: Extension;
  getCM: typeof getCM;
} {
  const autoInsert = Prec.highest(EditorView.domEventHandlers({
    compositionstart: (_event, view): boolean => {
      const cm = getCM(view);
      const state = cm?.state?.vim;
      if (!cm || !state || state.insertMode) return false;
      // Visual/operator-pending interpret `i` as part of a command. Esc first
      // normalizes every non-insert state and clears partial commands. It is
      // harmless in plain Normal mode.
      Vim.handleKey(cm, "<Esc>", "user");
      Vim.handleKey(cm, "i", "user");
      return false; // Never cancel the native composition.
    },
  }));
  return { extension: [autoInsert, vim()], getCM };
}
