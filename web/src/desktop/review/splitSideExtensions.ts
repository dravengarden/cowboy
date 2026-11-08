import { type Extension, RangeSetBuilder } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  gutter,
  GutterMarker,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import type { Theme } from "@mui/material/styles";
import { alpha } from "@mui/material/styles";
import type { SplitRowKind, SplitSide } from "./splitDiffModel";

const LINE: Partial<Record<SplitRowKind, Decoration>> = {
  add: Decoration.line({ class: "cowboy-split-add" }),
  delete: Decoration.line({ class: "cowboy-split-delete" }),
  filler: Decoration.line({ class: "cowboy-split-filler" }),
  hunk: Decoration.line({ class: "cowboy-split-hunk" }),
  meta: Decoration.line({ class: "cowboy-split-meta" }),
};

class Label extends GutterMarker {
  constructor(readonly label: string) {
    super();
  }
  override eq(other: GutterMarker): boolean {
    return other instanceof Label && other.label === this.label;
  }
  override toDOM(): Node {
    return document.createTextNode(this.label);
  }
}

function decorate(view: EditorView, kinds: readonly SplitRowKind[]): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to;) {
      const line = view.state.doc.lineAt(pos);
      const decoration = LINE[kinds[line.number - 1] ?? "context"];
      if (decoration) builder.add(line.from, line.from, decoration);
      pos = line.to + 1;
    }
  }
  return builder.finish();
}

/** One side of the split diff: row backgrounds and its file's line numbers. */
export function splitSideExtensions(theme: Theme, side: SplitSide): Extension[] {
  const dark = theme.palette.mode === "dark";
  const widest = side.labels.reduce((max, label) => label.length > max.length ? label : max, "0");
  return [
    ViewPlugin.fromClass(class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = decorate(view, side.kinds);
      }
      update(update: ViewUpdate): void {
        if (update.docChanged || update.viewportChanged) this.decorations = decorate(update.view, side.kinds);
      }
    }, { decorations: (plugin) => plugin.decorations }),
    gutter({
      class: "cm-lineNumbers",
      renderEmptyElements: true,
      lineMarker: (view, block) => {
        const label = side.labels[view.state.doc.lineAt(block.from).number - 1];
        return label ? new Label(label) : null;
      },
      initialSpacer: () => new Label(widest),
    }),
    EditorView.theme({
      ".cowboy-split-add": { backgroundColor: alpha(theme.palette.success.main, dark ? 0.16 : 0.12) },
      ".cowboy-split-delete": { backgroundColor: alpha(theme.palette.error.main, dark ? 0.16 : 0.1) },
      ".cowboy-split-filler": {
        backgroundImage: `repeating-linear-gradient(135deg, ${alpha(theme.palette.text.primary, 0.05)} 0 4px, transparent 4px 8px)`,
      },
      ".cowboy-split-hunk": { color: theme.palette.primary.main, backgroundColor: theme.palette.action.hover },
      ".cowboy-split-meta": { color: theme.palette.text.secondary, fontStyle: "italic" },
    }),
  ];
}
