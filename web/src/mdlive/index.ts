// mdlive — Obsidian-style CM6 markdown live-preview engine.
//
// Vendored from kenforthewin/atomic-editor@eba2066 (MIT). This barrel is the
// ONLY public entry point; the vendored files carry a DO-NOT-EDIT banner. See
// ./README.md for the API + invariants and ./SYNC.md for the upstream-sync
// procedure. v1 deliberately excludes the contenteditable surfaces (tables,
// wiki-links, image blocks) — see SYNC.md "Excluded files".
//
// LOCAL: this index.ts is cowboy-authored (not from upstream); upstream ships a
// React wrapper instead, which we don't vendor.

export { inlinePreview, type InlinePreviewConfig } from "./inline-preview.ts";
export {
  atomicEditorTheme,
  atomicMarkdownHighlight,
  atomicMarkdownSyntax,
} from "./atomic-theme.ts";
export {
  autoCloseCodeFence,
  autoCloseCodeFenceInput,
  extendEmphasisPair,
} from "./edit-helpers.ts";
