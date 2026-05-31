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
  const fontStack =
    'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';
  return EditorView.theme(
    {
      "&": {
        color: theme.palette.text.primary,
        backgroundColor: "transparent",
        // 14px desktop; 16px on touch so iOS Safari doesn't focus-zoom.
        fontSize: "14px",
      },
      "@media (pointer: coarse)": { "&": { fontSize: "16px" } },
      ".cm-content": {
        fontFamily: fontStack,
        padding: "8px 0",
        caretColor: accent,
        lineHeight: "1.5",
      },
      ".cm-scroller": { fontFamily: fontStack, lineHeight: "1.5" },
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
      // `@path` reference chips (see fileTokenWidget).
      ".cm-file-chip": {
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
