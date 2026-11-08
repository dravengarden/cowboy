import { type Extension, RangeSetBuilder } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin } from "@codemirror/view";
import type { Theme } from "@mui/material/styles";
import type { CodeDiagnostic } from "../../review/codeApi";

const CLASS = ["", "error", "warning", "info", "info"] as const;

function marks(view: EditorView, diagnostics: readonly CodeDiagnostic[]): DecorationSet {
  const doc = view.state.doc;
  const ranges = diagnostics.flatMap((diagnostic) => {
    if (diagnostic.start.row + 1 > doc.lines) return [];
    const line = doc.line(diagnostic.start.row + 1);
    const from = Math.min(line.to, line.from + diagnostic.start.column);
    const endLine = doc.line(Math.min(doc.lines, diagnostic.end.row + 1));
    let to = Math.min(endLine.to, endLine.from + diagnostic.end.column);
    if (to <= from) to = line.to;
    if (to <= from) return [];
    return [{ from, to, severity: CLASS[diagnostic.severity] ?? "info" }];
  }).sort((left, right) => left.from - right.from || left.to - right.to);
  const builder = new RangeSetBuilder<Decoration>();
  for (const range of ranges) {
    builder.add(range.from, range.to, Decoration.mark({ class: `cowboy-review-diagnostic-${range.severity}` }));
  }
  return builder.finish();
}

/** Wavy underlines for the language engine's diagnostics of a source file. */
export function diagnosticMarks(theme: Theme, diagnostics: readonly CodeDiagnostic[]): Extension[] {
  const wavy = (color: string) => ({ textDecoration: `underline wavy ${color}`, textDecorationSkipInk: "none" });
  return [
    ViewPlugin.define((view) => ({ decorations: marks(view, diagnostics) }), {
      decorations: (plugin) => plugin.decorations,
    }),
    EditorView.theme({
      ".cowboy-review-diagnostic-error": wavy(theme.palette.error.main),
      ".cowboy-review-diagnostic-warning": wavy(theme.palette.warning.main),
      ".cowboy-review-diagnostic-info": wavy(theme.palette.info.main),
    }),
  ];
}
