import type { ComposerEditorHandle } from "../ComposerEditor";
import type { EditorContext, EditorPort } from "./contract";

const editors = new Map<
  ComposerEditorHandle,
  { context: EditorContext; port: EditorPort; alive: () => boolean }
>();
let current: ComposerEditorHandle | null = null;
let dialog: ComposerEditorHandle | null = null;
const listeners = new Set<() => void>();
let tracksFocus = false;

/** Commands run from the palette or a toolbar after focus left the editor;
 * they target the editor the user last worked in, not the last one mounted. */
function trackEditorFocus(): void {
  if (tracksFocus || typeof document === "undefined") return;
  tracksFocus = true;
  document.addEventListener("focusin", () => {
    const focused = [...editors.keys()].find((entry) => entry.hasFocus());
    if (focused) current = focused;
  }, true);
}

export function bindEditorExtensions(
  editor: ComposerEditorHandle,
  context: EditorContext,
  ownsIme: () => boolean,
): () => void {
  trackEditorFocus();
  let alive = true;
  let lastText = editor.getValue();
  let revision = 0;
  let selection = editor.getSelection();
  const read = () => {
    if (!alive) return { text: lastText, revision, selection };
    const text = editor.getValue();
    if (text !== lastText) {
      revision++;
      lastText = text;
    }
    selection = editor.getSelection();
    return { text, revision, selection };
  };
  const port: EditorPort = {
    context,
    read,
    replaceSelection: (text, expected) => {
      if (!alive || ownsIme()) return false;
      const current = read();
      if (
        current.revision !== expected.revision ||
        current.text !== expected.text ||
        current.selection.anchor !== expected.selection.anchor ||
        current.selection.head !== expected.selection.head
      ) return false;
      editor.insertText(text, expected.selection);
      return true;
    },
    reveal: (offset) => {
      if (!alive || ownsIme()) return;
      const at = Math.max(0, Math.min(offset, editor.getValue().length));
      editor.focusSelection({ anchor: at, head: at });
      editor.revealSelection();
    },
  };
  const binding = { context, port, alive: () => alive };
  editors.set(editor, binding);
  current = editor;
  return () => {
    alive = false;
    if (editors.get(editor) === binding) editors.delete(editor);
    if (current === editor) current = null;
    if (dialog === editor) {
      dialog = null;
      for (const listener of listeners) listener();
    }
  };
}

export function openEditorExtensions(editor?: ComposerEditorHandle): void {
  const target = editor ??
    [...editors.keys()].find((entry) => entry.hasFocus()) ?? current;
  if (!target || !editors.has(target)) return;
  dialog = target;
  for (const listener of listeners) listener();
}
export function closeEditorExtensions(): void {
  dialog = null;
  for (const listener of listeners) listener();
}
export const editorExtensionDialog = {
  subscribe: (listener: () => void) => {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  },
  get: () => dialog ? editors.get(dialog) ?? null : null,
};

export function activeEditorExtensionPort(): EditorPort | null {
  const target = [...editors.keys()].find((entry) => entry.hasFocus()) ??
    current;
  return target ? editors.get(target)?.port ?? null : null;
}
