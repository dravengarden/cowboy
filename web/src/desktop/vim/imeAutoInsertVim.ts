import { Prec, type Extension } from "@codemirror/state";
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
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

const DIRECT_INSERT_KEYS = new Set(["i", "I", "a", "A", "o", "O", "s", "S", "C", "R"]);

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

  const commandSink = ViewPlugin.fromClass(class {
    readonly sink: HTMLDivElement;
    private cm: ReturnType<typeof getCM> = null;
    private focusFrame: number | null = null;
    private modeHandler: (() => void) | null = null;

    constructor(readonly view: EditorView) {
      this.sink = document.createElement("div");
      this.sink.tabIndex = 0;
      this.sink.setAttribute("role", "application");
      this.sink.setAttribute("aria-label", "Vim Normal mode command input");
      Object.assign(this.sink.style, {
        height: "1px",
        opacity: "0",
        overflow: "hidden",
        pointerEvents: "none",
        position: "absolute",
        width: "1px",
      });
      this.sink.addEventListener("focus", this.onSinkFocus);
      this.sink.addEventListener("blur", this.onSinkBlur);
      this.sink.addEventListener("keydown", this.onKeyDown);
      view.dom.append(this.sink);
      queueMicrotask(() => this.connect());
    }

    update(update: ViewUpdate): void {
      if (!this.cm) this.connect();
      if (update.focusChanged && update.view.hasFocus) this.focusSinkIfNormal();
    }

    destroy(): void {
      if (this.cm && this.modeHandler) {
        this.cm.off?.("vim-mode-change", this.modeHandler);
      }
      if (this.focusFrame !== null) cancelAnimationFrame(this.focusFrame);
      this.sink.removeEventListener("focus", this.onSinkFocus);
      this.sink.removeEventListener("blur", this.onSinkBlur);
      this.sink.removeEventListener("keydown", this.onKeyDown);
      this.view.dom.classList.remove("cm-vim-command-focused");
      this.sink.remove();
    }

    private connect(): void {
      if (this.cm) return;
      this.cm = getCM(this.view);
      if (!this.cm) return;
      this.modeHandler = (): void => this.syncFocusToMode();
      this.cm.on?.("vim-mode-change", this.modeHandler);
      this.focusSinkIfNormal();
    }

    private syncFocusToMode(): void {
      if (this.cm?.state?.vim?.insertMode) {
        this.focusEditorCaret();
      } else {
        this.focusSinkIfNormal();
      }
    }

    private focusEditorCaret(): void {
      this.view.dom.classList.remove("cm-vim-command-focused");
      this.view.focus();
      if (this.focusFrame !== null) cancelAnimationFrame(this.focusFrame);
      this.focusFrame = requestAnimationFrame(() => {
        this.focusFrame = null;
        if (!this.cm?.state?.vim?.insertMode) return;
        const head = this.view.state.selection.main.head;
        this.view.focus();
        this.view.dispatch({
          selection: { anchor: head },
          effects: EditorView.scrollIntoView(head),
        });
        const dom = this.view.domAtPos(head);
        const selection = window.getSelection();
        if (selection) selection.collapse(dom.node, dom.offset);
        this.view.requestMeasure();
      });
    }

    private focusSinkIfNormal(): void {
      if (!this.cm?.state?.vim?.insertMode && document.activeElement !== this.sink) {
        queueMicrotask(() => {
          if (!this.cm?.state?.vim?.insertMode) {
            this.sink.focus();
            this.view.dom.classList.add("cm-vim-command-focused");
          }
        });
      }
    }

    private readonly onKeyDown = (event: KeyboardEvent): void => {
      if (!this.cm || this.cm.state?.vim?.insertMode || event.isComposing) return;
      const key = normalModeKey(event);
      if (!key) return;
      event.preventDefault();
      event.stopPropagation();

      // Commands such as `a` and `o` move the selection or edit the document
      // before entering Insert. Run those with CodeMirror focused so it can
      // synchronize the native DOM selection/caret. The triggering keydown is
      // already consumed by the non-editable sink, so focusing here cannot
      // start an IME composition for this command.
      const changing = this.cm.state?.vim?.inputState?.operator === "change";
      if (DIRECT_INSERT_KEYS.has(key) || changing) this.view.focus();
      Vim.handleKey(this.cm, key, "user");
      this.syncFocusToMode();
    };

    private readonly onSinkFocus = (): void => {
      this.view.dom.classList.add("cm-vim-command-focused");
    };

    private readonly onSinkBlur = (): void => {
      this.view.dom.classList.remove("cm-vim-command-focused");
    };
  });

  return { extension: [autoInsert, vim(), commandSink], getCM };
}
