export const VIM_NATIVE_CARET_CLASS = "cm-vim-native-caret";

interface CursorOwnerClassList {
  toggle(token: string, force?: boolean): boolean;
}

interface CursorOwnerElement {
  classList: CursorOwnerClassList;
}

/**
 * Publish the cursor owner from Vim's synchronous mode state.
 *
 * CodeMirror's `.cm-focused` class describes browser focus, which WebView may
 * update after the Vim mode transition. Insert mode cannot wait for that paint:
 * the native caret owns the editable immediately and the old Vim cursor layer
 * must be suppressed in the same callback that changed modes.
 */
export function syncVimCursorOwnership(
  root: CursorOwnerElement,
  insertMode: boolean,
): void {
  root.classList.toggle(VIM_NATIVE_CARET_CLASS, insertMode);
}
