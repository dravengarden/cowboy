import type { Theme } from "@mui/material";
import { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";

// Map a MUI theme to a CodeMirror theme so the editor surface — and its
// autocomplete tooltip — read as native Material: palette, typography, the
// accent for caret/selection, light vs dark. Pure styling, no behavior. Driven
// from theme tokens so it tracks light/dark + accent changes automatically.
// (Task composer-cm6, plan Step 6 — the "yes, customizable to MUI" answer.)
export function cmTheme(theme: Theme): Extension {
  const dark = theme.palette.mode === "dark";
  const accent = theme.palette.primary.main;
  // Match the MUI theme's own typography so the editor text is indistinguishable
  // from a real MUI input.
  const fontStack =
    theme.typography.fontFamily ??
    'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';
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
      },
      ".cm-content": {
        fontFamily: fontStack,
        fontSize: "1rem",
        // The MUI-outline shell owns the padding (see ComposerEditor).
        padding: "0",
        caretColor: accent,
        lineHeight: "var(--cowboy-reading-line-height, 1.5)",
      },
      // iOS PWA repaint fix lives on the SCROLLER, not `.cm-content`. On
      // iPad/iPhone the composer's contenteditable sits inside the
      // `position: fixed` body (index.html, for keyboard handling); WebKit then
      // fails to invalidate the editable's paint rect on keystroke, so typed
      // text stays invisible until a later edit/scroll forces a repaint (delete
      // "reveals" it). A compositing layer in the subtree makes WebKit repaint
      // on every input — BUT promoting `.cm-content` ITSELF (the contenteditable)
      // breaks iOS's long-press text-interaction: the Copy/Paste callout +
      // selection loupe compute rects in the layer's coordinate space and
      // intermittently fail to attach, so long-press "often shows no paste menu".
      // Promoting the PARENT `.cm-scroller` instead keeps the repaint fix (the
      // editable still paints into the layer) while leaving the editable
      // un-transformed, so WebKit's editing UI attaches normally. translateZ(0)
      // is an identity transform (no visual shift), so CodeMirror's
      // getBoundingClientRect-based cursor/selection measurement is unaffected.
      ".cm-scroller": {
        fontFamily: fontStack,
        fontSize: "1rem",
        lineHeight: "var(--cowboy-reading-line-height, 1.5)",
        transform: "translateZ(0)",
      },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: accent },
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
    },
    { dark },
  );
}
