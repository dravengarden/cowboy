import type { Text } from "@codemirror/state";
import {
  type EditorView,
  gutter,
  GutterMarker,
  lineNumbers,
} from "@codemirror/view";
import { type DiffLineLabels, diffNewLineLabels } from "./diffSourceModel";

class LineLabel extends GutterMarker {
  constructor(readonly label: string) {
    super();
  }

  override eq(other: GutterMarker): boolean {
    return other instanceof LineLabel && other.label === this.label;
  }

  override toDOM(): Node {
    return document.createTextNode(this.label);
  }
}

const labelCache = new WeakMap<Text, DiffLineLabels>();

function labelsFor(doc: Text): DiffLineLabels {
  let labels = labelCache.get(doc);
  if (!labels) {
    labels = diffNewLineLabels(doc.toString());
    labelCache.set(doc, labels);
  }
  return labels;
}

/**
 * The Review line-number gutter. Source shows document lines; a diff shows
 * new-file lines so the number a reader taps matches the reference an Agent
 * receives. A tap reports the display row either way.
 */
export function reviewLineGutter(
  kind: "source" | "diff",
  onToggleLine: (row: number) => boolean,
) {
  const domEventHandlers = {
    click: (view: EditorView, block: { from: number }) =>
      onToggleLine(view.state.doc.lineAt(block.from).number),
  };
  if (kind === "source") return lineNumbers({ domEventHandlers });
  return gutter({
    // Reuse the line-number gutter's theme and selection styling.
    class: "cm-lineNumbers",
    // Keep one cell per row so every row stays a tap target.
    renderEmptyElements: true,
    lineMarker: (view, block) => {
      const row = view.state.doc.lineAt(block.from).number;
      const label = labelsFor(view.state.doc).labels[row - 1];
      return label ? new LineLabel(label) : null;
    },
    lineMarkerChange: (update) => update.docChanged,
    initialSpacer: (view) => new LineLabel(labelsFor(view.state.doc).widest),
    updateSpacer: (spacer, update) =>
      update.docChanged
        ? new LineLabel(labelsFor(update.state.doc).widest)
        : spacer,
    domEventHandlers,
  });
}
