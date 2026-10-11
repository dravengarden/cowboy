// Desktop Vim and inline images (mdlive/PITFALLS.md #0b).
//
// An image token is normally replaced by an atomic thumbnail. Vim does not
// move by atomic ranges: its Normal cursor sits ON a character and its
// motions set the selection directly, so `j`/`k`/`l` park the cursor on or
// inside the hidden token. There the block cursor has no glyph to cover (it
// measures zero wide at the token start, thumbnail-tall inside it) and `h`/`l`
// walk the hidden characters one by one.
//
// Like Obsidian's Live Preview, the token under the Vim cursor shows its
// source text and the picture renders below that line. The cursor is then
// always on real, visible text, and every motion and operator works on it.
// Without Vim nothing here applies: the token stays an atomic thumbnail.
import {
  Decoration,
  type EditorView,
  ViewPlugin,
  type WidgetType,
} from "@codemirror/view";
import {
  type EditorState,
  type Range,
  StateEffect,
  StateField,
} from "@codemirror/state";

/** The editor's Vim state: `null` when Vim is off. */
export type VimImageMode = "normal" | "insert" | null;

export const setVimImageMode = StateEffect.define<VimImageMode>();

export const vimImageMode = StateField.define<VimImageMode>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setVimImageMode)) value = effect.value;
    }
    return value;
  },
});

/** Whether the token at from..to shows its source instead of the thumbnail.
 * Outside Insert the cursor covers the character at the head, so the token
 * start counts, and so does any Visual selection reaching into the token. In
 * Insert the caret is between characters and only a caret already inside the
 * token (left there by a Normal command such as i) keeps it open, so typing
 * never lands in hidden text. */
export function imageSourceRevealed(
  state: EditorState,
  from: number,
  to: number,
): boolean {
  const mode = state.field(vimImageMode, false) ?? null;
  if (mode === null) return false;
  const selection = state.selection.main;
  if (mode === "insert") {
    return selection.head > from && selection.head < to;
  }
  return selection.empty
    ? selection.head >= from && selection.head < to
    : selection.from < to && selection.to > from;
}

/** The picture of a revealed token, on its own row below the source line. */
export function imageBelowSource(
  widget: WidgetType,
  lineEnd: number,
): Range<Decoration> {
  return Decoration.widget({ widget, block: true, side: 1 }).range(lineEnd);
}

interface VimModeSource {
  state?: { vim?: { insertMode?: boolean } | null };
  on?: (event: "vim-mode-change", handler: () => void) => void;
  off?: (event: "vim-mode-change", handler: () => void) => void;
}

/** Mirror the Vim adapter's mode into `vimImageMode`. The adapter keeps its
 * mode outside editor state and can change it without a transaction, so this
 * follows both its mode event and every view update. The write is deferred:
 * either source can run while the view is updating. */
export function vimImageModeSync(
  adapter: (view: EditorView) => VimModeSource | null,
) {
  return ViewPlugin.define((view) => {
    let source: VimModeSource | null = null;
    let live = true;
    let queued = false;
    const sync = (): void => {
      if (queued) return;
      queued = true;
      queueMicrotask(() => {
        queued = false;
        if (!live) return;
        if (!source) {
          source = adapter(view);
          source?.on?.("vim-mode-change", sync);
        }
        const vim = source?.state?.vim;
        const mode: VimImageMode = !vim
          ? null
          : vim.insertMode
          ? "insert"
          : "normal";
        if (mode !== view.state.field(vimImageMode, false)) {
          view.dispatch({ effects: setVimImageMode.of(mode) });
        }
      });
    };
    sync();
    return {
      update: sync,
      destroy(): void {
        live = false;
        source?.off?.("vim-mode-change", sync);
      },
    };
  });
}
