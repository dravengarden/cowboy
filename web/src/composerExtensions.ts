// Shared markdown live-preview + editing extension set for EVERY composer surface
// (desktop ComposerEditor, mobile compact input, fullscreen). Markdown is the
// literal editor value; the mdlive engine renders it inline and reveals raw
// markers on the active line (see web/src/mdlive/README.md).
//
// FAITHFUL port of atomic-editor's `AtomicCodeMirrorEditor` extension
// composition — the "Obsidian feel" lives in the EDITING + selection extensions,
// not just the decorations. We include everything it does EXCEPT items with a
// strong reason to drop (noted at the bottom).
import { markdown, markdownKeymap, markdownLanguage } from "@codemirror/lang-markdown";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { indentOnInput } from "@codemirror/language";
import { indentWithTab } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import { EditorView, highlightActiveLine, keymap } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import {
  atomicEditorTheme,
  atomicMarkdownSyntax,
  autoCloseCodeFence,
  extendEmphasisPair,
  inlinePreview,
} from "./mdlive";
// The engine's hide/reveal + layout CSS. Imported once here (this module is
// pulled in by every surface), so a host never has to remember to add it.
import "./mdlive/styles/inline-preview.css";

export interface LivePreviewOptions {
  /// Plain-click on a rendered link. Omit → the engine's default
  /// `window.open(url, "_blank", "noopener,noreferrer")`, which matches how
  /// cowboy opens links elsewhere (MarkdownImpl) and is handled by the Tauri
  /// shell's WKWebView. A platform shell can pass its own opener later.
  onLinkClick?: (url: string) => void;
}

// Returns the live-preview + markdown-editing extensions, mirroring atomic-editor.
// Append AFTER the host's own base extensions; the engine self-manages decoration
// precedence (its Enter handler is `Prec.highest`, owning list continuation, then
// falling through). `codeLanguages: []` = no embedded fenced-code grammars in v1.
export function livePreviewExtensions(
  opts: LivePreviewOptions = {},
): Extension[] {
  return [
    // iOS WebKit renders CM6's srcless `<img class="cm-widgetBuffer">` (the buffer
    // CM puts around widgets — the placeholder, and every hidden-marker widget) as
    // a tiny broken-image DOT. visibility:hidden kills the dot while keeping the
    // element's 0-width layout box, so cursor positioning around widgets is intact.
    EditorView.theme({ ".cm-widgetBuffer": { visibility: "hidden" } }),
    // Editor chrome that does NOT touch the native selection (safe on touch).
    highlightActiveLine(),
    indentOnInput(),
    // --- Obsidian-style bracket / emphasis / code-fence pairing ---
    closeBrackets(),
    extendEmphasisPair,
    autoCloseCodeFence,
    // Find-in-document (Mod-f), top panel — useful in the fullscreen long-form editor.
    search({ top: true }),
    // --- markdown language + GFM (source of the engine's syntax tree) ---
    markdown({ base: markdownLanguage, codeLanguages: [] }),
    markdownLanguage.data.of({
      closeBrackets: { brackets: ["(", "[", "{", "'", '"', "*", "_", "`"] },
    }),
    atomicMarkdownSyntax,
    atomicEditorTheme,
    keymap.of([...closeBracketsKeymap, ...searchKeymap, ...markdownKeymap, indentWithTab]),
    inlinePreview(opts),
    EditorView.lineWrapping,
  ];
  // DROPPED — drawSelection() / dropCursor() / rectangularSelection() /
  // allowMultipleSelections: these replace the native caret/selection with a
  // CM-drawn one, which kills iOS's long-press "Paste / Select" menu (a CONFIRMED
  // regression — the native edit menu needs the native selection). cowboy already
  // styles the caret via cmTheme (+ vim's block cursor). The visual-glitch fix
  // they'd bring on desktop isn't worth losing mobile paste; revisit desktop-only
  // if needed. Also still out (strong reasons): history/default/historyKeymap
  // (cowboy base provides — a 2nd history splits undo), tables/image-blocks/
  // wiki-links (the contenteditable IME surfaces), initialRevealField (no use case).
  // DROPPED from atomic's composition, each with a strong reason:
  //   • history() / historyKeymap / defaultKeymap — cowboy's ComposerEditor base
  //     already provides them; a second history() splits undo.
  //   • table-widget / image-blocks / wiki-links — the only contenteditable
  //     surfaces (the IME risk) and out of v1 scope. See mdlive/SYNC.md.
  //   • initialRevealField — a React-wrapper-local StateField for revealing an
  //     initial range on open (a search/deep-link use case cowboy doesn't have).
}
