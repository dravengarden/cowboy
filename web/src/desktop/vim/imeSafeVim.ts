import { Compartment, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { getCM, vim } from "@replit/codemirror-vim";

function imeEditable(editable: boolean): Extension {
  return [
    EditorView.editable.of(editable),
    EditorView.contentAttributes.of(editable
      ? { "data-vim-ime": "enabled" }
      : {
        tabindex: "0",
        "aria-readonly": "true",
        "data-vim-ime": "locked",
      }),
  ];
}

/** Desktop-only Vim runtime that removes the DOM IME target outside Insert. */
export function createImeSafeVim(): {
  extension: Extension;
  getCM: typeof getCM;
  syncIme: (view: EditorView, mode: string) => void;
} {
  const editable = new Compartment();
  let current = true;
  return {
    extension: [editable.of(imeEditable(true)), vim()],
    getCM,
    syncIme: (view, mode): void => {
      const normalized = mode.toLowerCase();
      const next = normalized === "insert" || normalized === "replace";
      if (next === current) return;
      current = next;
      const restoreFocus = view.hasFocus;
      queueMicrotask(() => {
        if (!view.dom.isConnected) return;
        view.dispatch({ effects: editable.reconfigure(imeEditable(next)) });
        if (restoreFocus && !view.hasFocus) view.contentDOM.focus();
      });
    },
  };
}
