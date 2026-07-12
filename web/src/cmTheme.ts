import { alpha, type Theme } from "@mui/material";
import { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";

// Map a MUI theme to a CodeMirror theme so the editor surface — and its
// autocomplete tooltip — read as native Material: palette, typography, the
// accent for caret/selection, light vs dark. Pure styling, no behavior. Driven
// from theme tokens so it tracks light/dark + accent changes automatically.
// (Task composer-cm6, plan Step 6 — the "yes, customizable to MUI" answer.)
export function cmTheme(theme: Theme, mono = false): Extension {
  const dark = theme.palette.mode === "dark";
  const accent = theme.palette.primary.main;
  // Match the MUI theme's own typography so the editor text is indistinguishable
  // from a real MUI input.
  const fontStack =
    theme.typography.fontFamily ??
    'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';
  // Zed-style monospace buffer font, used when `mono` is on (vim mode). CJK has
  // no glyphs in these faces, so Chinese falls back to the system CJK font —
  // same as Zed; this is why mono is gated to vim, not to normal prose chat.
  const monoStack =
    'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace';
  const editorFont = mono ? monoStack : fontStack;
  return EditorView.theme(
    {
      // `1rem` so the composer text TRACKS the reading font-size setting exactly
      // (useGlobalFontScale scales the root <html> font-size, so 1rem follows it
      // both up AND down — matching the transcript prose). No `max(16px, …)`
      // floor: the installed PWA disables focus-zoom via the viewport meta
      // (user-scalable=no), so the iOS-safe floor isn't needed and was the reason
      // the input ignored a sub-1.0 scale. line-height follows the reading
      // line-height setting via the CSS var (useGlobalFontScale sets it).
      "&": {
        color: theme.palette.text.primary,
        backgroundColor: "transparent",
        fontSize: "1rem",
        // Drive the mdlive engine's `--atomic-editor-*` colour tokens from the MUI
        // theme so the live-preview markdown (headings, links, inline code, quote
        // rails, selection…) tracks cowboy's LIGHT/DARK + accent automatically.
        // Without this the engine fell back to its built-in DARK defaults (its
        // `[data-theme=light] .atomic-cm-editor` overrides never matched cowboy's
        // editor), so markdown rendered dark-on-light in light mode. Typography
        // tokens are owned by cmTheme above; the `--atomic-editor-hl-*` code-syntax
        // tokens are skipped (cowboy passes `codeLanguages: []`, so no nested
        // grammar tokens are ever emitted).
        "--atomic-editor-fg": theme.palette.text.primary,
        "--atomic-editor-fg-muted": theme.palette.text.secondary,
        "--atomic-editor-fg-faint": theme.palette.text.disabled,
        "--atomic-editor-bg": theme.palette.background.paper,
        "--atomic-editor-bg-panel": theme.palette.background.paper,
        "--atomic-editor-bg-surface": dark
          ? alpha("#ffffff", 0.06)
          : alpha("#000000", 0.04),
        "--atomic-editor-border": theme.palette.divider,
        "--atomic-editor-accent": accent,
        "--atomic-editor-accent-bright": theme.palette.primary.light,
        "--atomic-editor-accent-soft": alpha(accent, 0.3),
        "--atomic-editor-link": accent,
        "--atomic-editor-link-hover": theme.palette.primary.light,
        "--atomic-editor-code-bg": alpha(accent, dark ? 0.16 : 0.08),
        "--atomic-editor-selection-bg": alpha(accent, dark ? 0.32 : 0.18),
        "--atomic-editor-initial-reveal-bg": alpha(accent, 0.16),
        "--atomic-editor-initial-reveal-bg-strong": alpha(accent, 0.3),
      },
      ".cm-content": {
        fontFamily: editorFont,
        fontSize: "1rem",
        // The MUI-outline shell owns the padding (see ComposerEditor).
        padding: "0",
        caretColor: accent,
        lineHeight: "var(--cowboy-reading-line-height, 1.5)",
        // `min-height: 100%` makes the contenteditable FILL the scroller, so the
        // empty area below the text is part of `.cm-content` — a long-press there
        // lands on the editable element, iOS resolves a caret (doc end), and the
        // Paste menu appears. This is exactly what Obsidian's mobile editor does
        // (verified against its `.cm-content` CSS: `min-height:100%`). Without it
        // the empty area is the non-editable `.cm-scroller`, so iOS finds no caret
        // target and shows nothing — the long-press only worked ON the text. (An
        // earlier note here claimed min-height BROKE the menu; that was a
        // misdiagnosis confounded by the now-removed drawSelection/translateZ
        // hacks — the transparent caret had no anchor regardless of the fill.)
        // No effect on the compact composer: its scroller is content-height, so
        // 100% resolves to that — it only fills the fixed-height fullscreen editor.
        minHeight: "100%",
      },
      ".cm-scroller": {
        fontFamily: editorFont,
        fontSize: "1rem",
        lineHeight: "var(--cowboy-reading-line-height, 1.5)",
        // Deliberately no compositing layer. The current normal-flow editor has
        // no fixed-body repaint bug, and the retired translateZ workaround
        // interfered with iOS native text interaction. See mdlive/PITFALLS.md.
      },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: accent },
      // Vim block ("fat") cursor → Zed look. @replit/codemirror-vim defaults to a
      // pink #ff9696 block; recolour to a solid accent block with the glyph
      // inverted onto it (primary.contrastText → always readable on the accent).
      // Unfocused: a hollow accent outline (the package blanks the glyph there).
      ".cm-fat-cursor": {
        background: `${accent} !important`,
        color: `${theme.palette.primary.contrastText} !important`,
      },
      "&:not(.cm-focused) .cm-fat-cursor": {
        background: "none !important",
        outline: `solid 1px ${accent}`,
      },
      // Don't blink the block cursor (Zed keeps it solid). The vim cursor layer
      // blinks via a JS-set CSS animation on `.cm-vimCursorLayer`; cancel it.
      ".cm-cursorLayer.cm-vimCursorLayer": { animation: "none !important" },
      ".cm-placeholder": { color: theme.palette.text.disabled },
      "&.cm-focused": { outline: "none" },
      // Selection — use the MUI selection token in both the focused and
      // unfocused states so it matches a real input.
      "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
        { backgroundColor: theme.palette.action.selected },
      // Autocomplete popup → MUI Paper.
      ".cm-tooltip": {
        backgroundColor: theme.palette.background.paper,
        color: theme.palette.text.primary,
        border: `1px solid ${theme.palette.divider}`,
        borderRadius: "8px",
        boxShadow: theme.shadows[6],
        overflow: "hidden",
      },
      ".cm-tooltip.cm-tooltip-autocomplete > ul": {
        fontFamily: fontStack,
        maxHeight: "15rem",
      },
      ".cm-tooltip.cm-tooltip-autocomplete > ul > li": {
        padding: "6px 12px",
        lineHeight: "1.4",
      },
      ".cm-tooltip-autocomplete ul li[aria-selected]": {
        backgroundColor: theme.palette.action.hover,
        color: theme.palette.text.primary,
      },
      ".cm-completionLabel": { fontFamily: fontStack },
      ".cm-completionDetail": {
        color: theme.palette.text.secondary,
        fontStyle: "normal",
        marginLeft: "8px",
        fontSize: "0.85em",
      },
      // `@path` / `/skill` chips (see fileTokenWidget).
      ".cm-token-chip": {
        display: "inline-flex",
        alignItems: "center",
        padding: "0 6px",
        margin: "0 1px",
        borderRadius: "6px",
        backgroundColor: theme.palette.action.selected,
        color: theme.palette.text.primary,
        fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
        fontSize: "0.9em",
        lineHeight: "1.5",
        whiteSpace: "nowrap",
        cursor: "default",
      },
      // GFM task checkbox. The vendored mdlive emits a NATIVE <input
      // type=checkbox>, which on iOS WebKit renders as a gray hollow CIRCLE
      // (accent-color only tints the checked state) — out of place next to the
      // Material composer ("样式有点丑"). Override to an Obsidian-style rounded
      // square: empty box w/ a muted border when unchecked, accent fill + white
      // tick when done. `input.` + the CM-theme class prefix beats the vendored
      // plain-class rule, so this stays in OUR file (mdlive CSS is DO-NOT-EDIT).
      // Footprint stays 0.9em box + 0.3em gap so the shared list alcove math
      // (ALCOVE_EM in inline-preview.ts) is untouched.
      "input.cm-atomic-task-checkbox": {
        appearance: "none",
        WebkitAppearance: "none",
        boxSizing: "border-box",
        width: "0.9em",
        height: "0.9em",
        margin: "0 0.3em 0 0",
        verticalAlign: "-0.15em",
        position: "relative",
        border: `1.5px solid ${theme.palette.text.disabled}`,
        borderRadius: "0.28em",
        backgroundColor: "transparent",
        cursor: "pointer",
        transition: "background-color 0.12s ease, border-color 0.12s ease",
      },
      "input.cm-atomic-task-checkbox:hover": { borderColor: accent },
      "input.cm-atomic-task-checkbox:checked": {
        backgroundColor: accent,
        borderColor: accent,
      },
      // The tick — a rotated bottom-right border corner, classic CSS checkmark.
      "input.cm-atomic-task-checkbox:checked::after": {
        content: '""',
        position: "absolute",
        left: "0.28em",
        top: "0.08em",
        width: "0.2em",
        height: "0.42em",
        border: `solid ${theme.palette.primary.contrastText}`,
        borderWidth: "0 0.12em 0.12em 0",
        transform: "rotate(45deg)",
      },
      // `==highlight==` (composerHighlight.ts + the mdlive node-class entries).
      // A yellow marker like Obsidian — theme-tuned: a solid warm yellow on
      // light (default dark text stays legible), a translucent amber on dark
      // (inherited light text stays legible). box-decoration-break so a wrapped
      // highlight keeps its rounding on each line fragment.
      ".cm-atomic-highlight": {
        backgroundColor: dark ? "rgba(255, 213, 79, 0.26)" : "rgba(255, 235, 130, 0.9)",
        color: dark ? "inherit" : "#000",
        borderRadius: "3px",
        padding: "0 1px",
        WebkitBoxDecorationBreak: "clone",
        boxDecorationBreak: "clone",
      },
    },
    { dark },
  );
}
