import { Prec, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { getCM, Vim, vim } from "@replit/codemirror-vim";

const CODE_KEYS: Readonly<Record<string, string>> = {
  Backquote: "`",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Digit0: "0",
  Digit1: "1",
  Digit2: "2",
  Digit3: "3",
  Digit4: "4",
  Digit5: "5",
  Digit6: "6",
  Digit7: "7",
  Digit8: "8",
  Digit9: "9",
  Equal: "=",
  Minus: "-",
  Period: ".",
  Quote: "'",
  Semicolon: ";",
  Slash: "/",
};

function normalModeKey(event: KeyboardEvent): string | null {
  if (event.metaKey || event.ctrlKey || event.altKey) return null;
  if (/^Key[A-Z]$/.test(event.code)) {
    const letter = event.code.slice(3).toLowerCase();
    return event.shiftKey ? letter.toUpperCase() : letter;
  }
  const key = CODE_KEYS[event.code];
  if (!key) return null;
  if (!event.shiftKey) return key;
  return {
    "`": "~",
    "1": "!",
    "2": "@",
    "3": "#",
    "4": "$",
    "5": "%",
    "6": "^",
    "7": "&",
    "8": "*",
    "9": "(",
    "0": ")",
    "-": "_",
    "=": "+",
    "[": "{",
    "]": "}",
    "\\": "|",
    ";": ":",
    "'": '"',
    ",": "<",
    ".": ">",
    "/": "?",
  }[key] ?? key;
}

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
    keydown: (event, view): boolean => {
      const cm = getCM(view);
      const state = cm?.state?.vim;
      if (!cm || !state || state.insertMode || event.isComposing) return false;
      const key = normalModeKey(event);
      if (!key) return false;

      // Prevent the active system IME from turning a Vim command into marked
      // text. `code` represents the physical Latin command key even when
      // macOS reports `key=Process`/starts a CJK composition.
      event.preventDefault();
      Vim.handleKey(cm, key, "user");
      return true;
    },
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
