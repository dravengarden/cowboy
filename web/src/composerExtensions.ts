// Shared markdown live-preview extension set for EVERY composer surface (desktop
// ComposerEditor, mobile compact input, fullscreen). Markdown is the literal
// editor value; the mdlive engine renders it inline and reveals raw markers on
// the active line (see web/src/mdlive/README.md). Factoring it here keeps the
// four hosts mounting ONE identical engine config — switching surfaces never
// re-serializes the text.
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import type { Extension } from "@codemirror/state";
import {
  atomicEditorTheme,
  atomicMarkdownSyntax,
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

// Returns the live-preview extensions in mount order. Append AFTER the host's
// own language-agnostic extensions; the engine self-manages precedence (its
// Enter handler is `Prec.highest`). `codeLanguages: []` = no embedded fenced-code
// grammars in v1 (keeps the dep tree + bundle small).
export function livePreviewExtensions(
  opts: LivePreviewOptions = {},
): Extension[] {
  return [
    markdown({ base: markdownLanguage, codeLanguages: [] }),
    atomicEditorTheme,
    atomicMarkdownSyntax,
    inlinePreview(opts),
  ];
}
