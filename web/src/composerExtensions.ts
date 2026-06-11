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
import { Highlight } from "./composerHighlight";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { indentOnInput } from "@codemirror/language";
import { indentWithTab } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import {
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  keymap,
} from "@codemirror/view";
import { isNativeShell } from "./nativeShell";
import { type Extension, Prec } from "@codemirror/state";
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
  const nativeShell = isNativeShell();
  return [
    // iOS WebKit renders CM6's srcless `<img class="cm-widgetBuffer">` (the buffer
    // CM puts around widgets — the placeholder, and every hidden-marker widget) as
    // a tiny broken-image DOT. visibility:hidden kills the dot while keeping the
    // element's 0-width layout box, so cursor positioning around widgets is intact.
    EditorView.theme({ ".cm-widgetBuffer": { visibility: "hidden" } }),
    // CARET + SELECTION — `drawSelection()` in the PWA ONLY; the NATIVE shell uses
    // the pure NATIVE caret/selection.
    //
    // THE iOS long-press Paste-menu fix (device-confirmed 2026-06-11, the whole
    // saga). drawSelection's `hideNativeSelection` forces `caret-color: transparent`
    // — and the iOS no-selection callout (Paste | Select | …) ANCHORS to the native
    // caret. With it transparent, the menu flickers up and immediately dismisses on
    // a real device (AXe on the sim couldn't catch it — synthetic touch isn't a
    // UIKit gesture). Obsidian's mobile editor proves the target state: it runs
    // drawSelection too BUT keeps the native caret VISIBLE (`caret-color
    // rgb(34,34,34)`, CDP-confirmed) so the menu has an anchor. We get the same
    // visible-native-caret end state more simply by dropping drawSelection in the
    // shell. Paired with `scrollPastEnd` (ComposerEditor fill mode) so a long-press
    // in the empty area snaps the caret to the line end (a real position), the menu
    // now has BOTH a real position AND a visible caret to anchor to.
    //
    // The PWA keeps drawSelection: it needs the `translateZ(0)` repaint layer
    // (cmTheme, pitfall #2) which mis-paints the native caret/selection, so there it
    // must draw its own. The `.cm-composing` dance + dropCursor only matter with the
    // drawn caret, so they're PWA-only too.
    ...(nativeShell ? [] : [
      drawSelection(),
      dropCursor(),
      EditorView.theme({
        "&.cm-composing .cm-content, &.cm-composing .cm-line": {
          caretColor: "var(--atomic-editor-accent-bright, #a78bfa) !important",
        },
        "&.cm-composing .cm-cursorLayer": { visibility: "hidden" },
      }),
      EditorView.domEventHandlers({
        compositionstart: (_e, view): boolean => {
          view.dom.classList.add("cm-composing");
          return false;
        },
        compositionend: (_e, view): boolean => {
          view.dom.classList.remove("cm-composing");
          return false;
        },
        // A composition can be interrupted WITHOUT a compositionend (focus loss, the
        // native photo picker stealing focus). A blur tears the styling down so the
        // caret can't get stuck in the native state.
        blur: (_e, view): boolean => {
          view.dom.classList.remove("cm-composing");
          return false;
        },
      }),
    ]),
    highlightActiveLine(),
    indentOnInput(),
    // --- Obsidian-style bracket / emphasis / code-fence pairing ---
    closeBrackets(),
    extendEmphasisPair,
    autoCloseCodeFence,
    // Find-in-document (Mod-f), top panel — useful in the fullscreen long-form editor.
    search({ top: true }),
    // --- markdown language + GFM (source of the engine's syntax tree) ---
    // `extensions: [Highlight]` teaches lezer `==text==` (composerHighlight.ts) —
    // GFM has no highlight rule. mdlive renders it via the node-class entries.
    markdown({ base: markdownLanguage, codeLanguages: [], extensions: [Highlight] }),
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
    keymap.of([...searchKeymap, ...markdownKeymap, indentWithTab]),
    inlinePreview(opts),
    EditorView.lineWrapping,
  ];
  // Every add/drop here, and every iOS pitfall it touches, is documented in
  // web/src/mdlive/PITFALLS.md — READ IT before changing this set. The cardinal
  // rule: these CM6 extensions are COUPLED on iOS WebKit (caret ↔ IME ↔ the
  // native paste menu ↔ widget render). Do NOT toggle one to chase a single
  // symptom; align with Obsidian and re-verify the WHOLE iOS matrix.
  //
  // DROPPED from atomic's composition, each with a strong reason:
  //   • history() / historyKeymap / defaultKeymap — cowboy's ComposerEditor base
  //     already provides them; a second history() splits undo.
  //   • table-widget / image-blocks / wiki-links — the only contenteditable
  //     surfaces (the IME risk) and out of v1 scope. See mdlive/SYNC.md.
  //   • rectangularSelection() / allowMultipleSelections — desktop multi-cursor;
  //     re-add desktop-only if ever wanted (PITFALLS.md inventory).
  //   • initialRevealField — a React-wrapper-local StateField for revealing an
  //     initial range on open (a search/deep-link use case cowboy doesn't have).
}
