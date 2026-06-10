// Shared markdown live-preview + editing extension set for EVERY composer surface
// (desktop ComposerEditor, mobile compact input, fullscreen). Markdown is the
// literal editor value; the mdlive engine renders it inline and reveals raw
// markers on the active line (see web/src/mdlive/README.md).
//
// This is a faithful port of atomic-editor's `AtomicCodeMirrorEditor` extension
// composition (the "Obsidian feel" lives in the EDITING extensions, not just the
// decorations): bracket/emphasis auto-pairing, code-fence auto-close, the
// markdown keymap (list/quote continuation on Enter), indent-on-input, and the
// cursor/active-line visuals the atomic theme styles. We OMIT only what cowboy's
// ComposerEditor base already provides (`history`, `lineWrapping`, its own
// `historyKeymap`/`defaultKeymap`/completion keymaps + send-chord handler) and
// what v1 excludes (tables, image blocks, wiki-links, find-in-document, and the
// React-wrapper-only `initialRevealField`).
import { markdown, markdownKeymap, markdownLanguage } from "@codemirror/lang-markdown";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { indentOnInput } from "@codemirror/language";
import { EditorView, keymap } from "@codemirror/view";
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

// Returns the live-preview + markdown-editing extensions in mount order, matching
// atomic-editor's composition. Append AFTER the host's own base extensions; the
// engine self-manages decoration precedence (its Enter handler is `Prec.highest`,
// so it beats `markdownKeymap`'s Enter for tight-list continuation, then falls
// through). `codeLanguages: []` = no embedded fenced-code grammars in v1.
export function livePreviewExtensions(
  opts: LivePreviewOptions = {},
): Extension[] {
  return [
    // Auto-indent on input (e.g. continuing an indented list block).
    indentOnInput(),
    // --- Obsidian-style bracket / emphasis / code-fence pairing (the editing
    // "logic" the look-alike decorations alone don't provide) ---
    closeBrackets(),
    extendEmphasisPair,
    autoCloseCodeFence,
    // --- The markdown language + GFM, the source of the syntax tree the engine
    // reads (Task / Strikethrough / autolinks need `base: markdownLanguage`) ---
    markdown({ base: markdownLanguage, codeLanguages: [] }),
    // Extend closeBrackets to markdown's symmetric delimiters — typing `*`/`_`/`` ` ``
    // auto-pairs, and extendEmphasisPair grows the pair as you type inside it.
    markdownLanguage.data.of({
      closeBrackets: { brackets: ["(", "[", "{", "'", '"', "*", "_", "`"] },
    }),
    atomicMarkdownSyntax,
    atomicEditorTheme,
    // closeBrackets (Backspace-over-pair) + markdown keybindings. NOT history/
    // default keymaps (cowboy's base already registers those), and NOT
    // `indentWithTab` — Tab is the completion-accept / focus key in this chat box,
    // not an indent key.
    keymap.of([...closeBracketsKeymap, ...markdownKeymap]),
    // The live-preview decorations themselves (+ the markdown-aware Enter, which
    // is Prec.highest so it owns list continuation, beating markdownKeymap's Enter).
    inlinePreview(opts),
    EditorView.lineWrapping,
  ];
  // DELIBERATELY NOT ported from atomic-editor's AtomicCodeMirrorEditor here:
  //   drawSelection() / dropCursor() / highlightActiveLine() /
  //   allowMultipleSelections — these are cursor/selection CHROME, not editing
  //   logic. drawSelection in particular replaces the native caret, which is
  //   exactly the kind of change that historically stranded iOS pinyin IME (the
  //   project's hard constraint), and cowboy already styles the caret via cmTheme
  //   (+ the vim block cursor). search()/tables/imageBlocks/initialRevealField are
  //   excluded too (v1 scope / not a chat-composer need). See mdlive/SYNC.md.
}
