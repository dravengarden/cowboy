import { type Extension, RangeSetBuilder } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import type { Theme } from "@mui/material/styles";
import { alpha } from "@mui/material/styles";

const ADDED = Decoration.line({ class: "cowboy-review-added" });
const REMOVED = Decoration.line({ class: "cowboy-review-removed" });
const HUNK = Decoration.line({ class: "cowboy-review-hunk" });
const META = Decoration.line({ class: "cowboy-review-meta" });

/** The structural role of one unified-diff line. */
export function diffLineRole(text: string): "added" | "removed" | "hunk" | "meta" | null {
  if (text.startsWith("@@")) return "hunk";
  if (text.startsWith("+++") || text.startsWith("---") || text.startsWith("diff --git") ||
    text.startsWith("index ") || text.startsWith("\\")) return "meta";
  if (text.startsWith("+")) return "added";
  if (text.startsWith("-")) return "removed";
  return null;
}

function decorate(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to;) {
      const line = view.state.doc.lineAt(pos);
      const role = diffLineRole(line.text);
      if (role) {
        builder.add(line.from, line.from, role === "added" ? ADDED : role === "removed" ? REMOVED : role === "hunk" ? HUNK : META);
      }
      pos = line.to + 1;
    }
  }
  return builder.finish();
}

/** Line backgrounds for a unified diff, computed for the visible range only. */
export function reviewDiffDecorations(theme: Theme): Extension {
  const dark = theme.palette.mode === "dark";
  return [
    ViewPlugin.fromClass(class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = decorate(view);
      }
      update(update: ViewUpdate): void {
        if (update.docChanged || update.viewportChanged) this.decorations = decorate(update.view);
      }
    }, { decorations: (plugin) => plugin.decorations }),
    EditorView.theme({
      ".cowboy-review-added": { backgroundColor: alpha(theme.palette.success.main, dark ? 0.16 : 0.12) },
      ".cowboy-review-removed": { backgroundColor: alpha(theme.palette.error.main, dark ? 0.16 : 0.1) },
      ".cowboy-review-hunk": {
        color: theme.palette.primary.main,
        backgroundColor: theme.palette.action.hover,
      },
      ".cowboy-review-meta": { color: theme.palette.text.secondary },
    }),
  ];
}
