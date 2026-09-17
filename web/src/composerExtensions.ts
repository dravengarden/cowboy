// Shared markdown live-preview + editing extension set for EVERY composer surface
// (desktop ComposerEditor, mobile compact input, fullscreen). Markdown is the
// literal editor value; the mdlive engine renders it inline and reveals raw
// markers on the active line (see web/src/mdlive/README.md).
//
// FAITHFUL port of atomic-editor's `AtomicCodeMirrorEditor` extension
// composition — the "Obsidian feel" lives in the EDITING + selection extensions,
// not just the decorations. We include everything it does EXCEPT items with a
// strong reason to drop (noted at the bottom).
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { Highlight } from "./composerHighlight";
import { closeBracketsKeymap } from "@codemirror/autocomplete";
import { indentOnInput, indentUnit } from "@codemirror/language";
import { obsidianMarkdownKeymap } from "./composer/markdownEditingCommands";
import {
  EditorView,
  highlightActiveLine,
  keymap,
} from "@codemirror/view";
import { type Extension, Prec } from "@codemirror/state";
import { openExternalUrl } from "./openExternal";
import {
  atomicEditorTheme,
  atomicMarkdownSyntax,
  inlinePreview,
} from "./mdlive";
import { obsidianAutoPair } from "./composer/obsidianAutoPair";
// The engine's hide/reveal + layout CSS. Imported once here (this module is
// pulled in by every surface), so a host never has to remember to add it.
import "./mdlive/styles/inline-preview.css";

// Obsidian sets `EditorView.EDIT_CONTEXT = false` before creating any editor.
// @codemirror/view otherwise routes Android Chrome input through the
// EditContext API, where composition events fire on the EditContext object
// instead of the DOM. Cowboy's composition holds (the textarea↔CM6 host swap,
// the soft-keyboard Backspace chain) listen on the DOM, so keep Android on the
// same contenteditable input path as iOS and Obsidian. Global by design.
(EditorView as unknown as { EDIT_CONTEXT?: boolean }).EDIT_CONTEXT = false;

export interface LivePreviewOptions {
  /// Obsidian's Source mode: keep the markdown LITERAL — no inline
  /// rendering, no hidden markers — while every other editing behaviour
  /// stays identical. Off (live preview) is the default. See
  /// composerSourceMode.ts.
  sourceMode?: boolean;
  /// Plain-click on a rendered link. Omit → the engine's default
  /// `window.open(url, "_blank", "noopener,noreferrer")`, which matches how
  /// cowboy opens links elsewhere (MarkdownImpl) and is handled by the Tauri
  /// shell's WKWebView. A platform shell can pass its own opener later.
  onLinkClick?: (url: string) => void;
}

// Returns the live-preview + markdown-editing extensions. Append AFTER the
// host's own base extensions: the host's Prec.high image-line Enter and
// Backspace chain must run before Obsidian's list keymap here.
// `codeLanguages: []` = no embedded fenced-code grammars in v1.
export function livePreviewExtensions(
  opts: LivePreviewOptions = {},
): Extension[] {
  return [
    // iOS WebKit renders CM6's srcless `<img class="cm-widgetBuffer">` (the buffer
    // CM puts around widgets — the placeholder, and every hidden-marker widget) as
    // a tiny broken-image DOT. visibility:hidden kills the dot while keeping the
    // element's 0-width layout box, so cursor positioning around widgets is intact.
    EditorView.theme({ ".cm-widgetBuffer": { visibility: "hidden" } }),
    // CARET + SELECTION: pure NATIVE caret/selection — NO drawSelection. The iOS
    // long-press Paste/Select callout ANCHORS to the native caret, so it must stay
    // visible (Obsidian's mobile editor does the same). drawSelection's
    // `hideNativeSelection` forced `caret-color: transparent`, which made the
    // callout flicker up and dismiss on a real device. It — plus dropCursor and the
    // `.cm-composing` dance — only ever existed to compensate for the PWA's
    // `translateZ(0)` repaint layer, which the native shell (normal flow) no longer
    // has. All removed at the root. Fill editors resolve their visible canvas to
    // the real `.cm-content`, so empty-area long presses retain a native anchor.
    // Do NOT re-add drawSelection / dropCursor / the composition dance.
    highlightActiveLine(),
    indentOnInput(),
    // --- Obsidian bracket / emphasis / code-fence pairing ---
    // Obsidian ships its own closeBrackets fork for Markdown: same-character
    // markers pair only between whitespace, tracked closers are stepped over,
    // three backticks open a fence, and `= ~ $ %` wrap a selection. It replaces
    // upstream closeBrackets() plus the vendored extendEmphasisPair and
    // autoCloseCodeFence, which together turned typed `**bold**` into
    // `**bold******`. See composer/obsidianAutoPair.ts and PITFALLS.md.
    obsidianAutoPair,
    // --- markdown language + GFM (source of the engine's syntax tree) ---
    // `extensions: [Highlight]` teaches lezer `==text==` (composerHighlight.ts) —
    // GFM has no highlight rule. mdlive renders it via the node-class entries.
    // `addKeymap: false`: Obsidian's own Enter / Shift-Enter / Tab below replace
    // lang-markdown's loose-list continuation and markup-aware Backspace.
    // Obsidian deletes list markup one character at a time.
    markdown({
      base: markdownLanguage,
      codeLanguages: [],
      extensions: [Highlight],
      addKeymap: false,
    }),
    // Obsidian's default `useTab`: a tab nests `1.` items as well as bullets.
    indentUnit.of("\t"),
    // Read by closeBracketsKeymap's empty-pair Backspace only.
    markdownLanguage.data.of({
      closeBrackets: { brackets: ["(", "[", "{", "'", '"', "*", "_", "`"] },
    }),
    atomicMarkdownSyntax,
    atomicEditorTheme,
    // closeBracketsKeymap's Backspace deletes an EMPTY pair as a unit (`*|*`,
    // `**|**`, `` `|` ``, `(|)`, …) — Obsidian's "delete front removes back too".
    // It MUST out-rank cowboy's defaultKeymap `deleteCharBackward` (which only
    // deletes one char, orphaning the closer), so wrap it Prec.high. (cowboy's
    // own Prec.high token-Backspace runs first but no-ops outside a token.)
    Prec.high(keymap.of(closeBracketsKeymap)),
    // Obsidian's list Enter, Shift-Enter and quote-aware Tab / Shift-Tab, in
    // live preview and Source mode alike. Prec.high but later than the
    // composer's own Prec.high image-line Enter and Backspace chain.
    obsidianMarkdownKeymap,
    // LIVE PREVIEW vs SOURCE MODE. Source mode leaves the whole decoration
    // engine OUT rather than suppressing it from the outside: mdlive has no
    // "off" switch, and a half-mounted decoration layer is exactly the kind of
    // coupled state PITFALLS.md forbids. Everything else — the syntax tree, the
    // highlight colours, pairing, wrapping, and cowboy's own @-token and
    // attachment widgets — is identical in both modes, so toggling can never
    // change the document.
    //
    // Enter is identical in both modes, as in Obsidian: the mode only changes
    // decorations. (mdlive's tight-list Enter was removed; see SYNC.md.)
    ...(opts.sourceMode
      ? [
        // Marks the editable for tests and for anything that needs to style
        // the raw surface. Decorative only — the mode change is announced by
        // the control the user just operated.
        EditorView.contentAttributes.of({ "data-composer-source": "true" }),
      ]
      : [
        inlinePreview({
          onLinkClick: opts.onLinkClick ?? openExternalUrl,
        }),
      ]),
    EditorView.lineWrapping,
  ];
  // Every add/drop here, and every iOS pitfall it touches, is documented in
  // web/src/mdlive/PITFALLS.md — READ IT before changing this set. The cardinal
  // rule: these CM6 extensions are COUPLED on iOS WebKit (caret ↔ IME ↔ the
  // native paste menu ↔ widget render). Do NOT toggle one to chase a single
  // symptom; align with Obsidian and re-verify the WHOLE iOS matrix.
  //
  // DROPPED from atomic's composition, each with a strong reason:
  //   • search() / searchKeymap — stock CM6 Find is browser chrome. Desktop
  //     already refuses Mod+F as a Cowboy command; do not let a vendored keymap
  //     reopen that panel.
  //   • history() / historyKeymap / defaultKeymap — cowboy's ComposerEditor base
  //     already provides them; a second history() splits undo.
  //   • table-widget / image-blocks / wiki-links — the only contenteditable
  //     surfaces (the IME risk) and out of v1 scope. See mdlive/SYNC.md.
  //   • rectangularSelection() / allowMultipleSelections — desktop multi-cursor;
  //     re-add desktop-only if ever wanted (PITFALLS.md inventory).
  //   • initialRevealField — a React-wrapper-local StateField for revealing an
  //     initial range on open (a search/deep-link use case cowboy doesn't have).
}
