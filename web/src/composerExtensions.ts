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
  autoCloseCodeFence,
  extendEmphasisPair,
  inlinePreview,
} from "./mdlive";
// The engine's hide/reveal + layout CSS. Imported once here (this module is
// pulled in by every surface), so a host never has to remember to add it.
import "./mdlive/styles/inline-preview.css";

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
    // --- Obsidian-style bracket / emphasis / code-fence pairing ---
    closeBrackets(),
    extendEmphasisPair,
    autoCloseCodeFence,
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
    keymap.of([...markdownKeymap, indentWithTab]),
    // LIVE PREVIEW vs SOURCE MODE. Source mode leaves the whole decoration
    // engine OUT rather than suppressing it from the outside: mdlive has no
    // "off" switch, and a half-mounted decoration layer is exactly the kind of
    // coupled state PITFALLS.md forbids. Everything else — the syntax tree, the
    // highlight colours, pairing, wrapping, and cowboy's own @-token and
    // attachment widgets — is identical in both modes, so toggling can never
    // change the document.
    //
    // The one behavioural difference is Enter. mdlive owns a Prec.highest
    // tight-list continuation (inline-preview.ts `insertTightListItem`),
    // which exists because loose and tight lists LOOK identical once rendered.
    // With the engine out, @codemirror/lang-markdown's own
    // `insertNewlineContinueMarkup` takes over — it still continues bullets,
    // ordered items and tasks, and its loose-list blank line is the honest
    // result on a surface where the blank lines are visible. Do not
    // re-implement the tight variant here; that would fork vendored logic
    // (mdlive/SYNC.md).
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
