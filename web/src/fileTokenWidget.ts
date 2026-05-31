import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { RangeSetBuilder } from "@codemirror/state";

// Render completed `@path` references as Material chips. Display-only: the
// underlying doc text stays `@path`, so the prompt sent to the agent is
// unchanged — only the rendering differs. The token the caret is in is left as
// plain text (it's the one being typed/edited); everything else becomes an
// atomic chip, so a single backspace removes the whole reference. (Plan Step 10.)
class FileChipWidget extends WidgetType {
  constructor(private readonly label: string) {
    super();
  }
  override eq(other: FileChipWidget): boolean {
    return other.label === this.label;
  }
  override toDOM(): HTMLElement {
    const span = document.createElement("span");
    span.className = "cm-file-chip";
    span.textContent = this.label;
    return span;
  }
  override ignoreEvent(): boolean {
    return false;
  }
}

// `@` at start-of-input or after whitespace, then a run of non-space chars.
const MENTION_RE = /(?:^|\s)(@\S+)/g;

function buildChips(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const caret = view.state.selection.main.head;
  for (const { from, to } of view.visibleRanges) {
    const text = view.state.doc.sliceString(from, to);
    MENTION_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = MENTION_RE.exec(text)) !== null) {
      const token = m[1];
      if (token === undefined) continue;
      const start = from + m.index + (m[0].length - token.length);
      const end = start + token.length;
      // Guard slice-boundary false positives (e.g. an email `a@b`): the char
      // just before `@` must be whitespace or the very start of the doc.
      if (start > 0) {
        const prev = view.state.doc.sliceString(start - 1, start);
        if (prev && !/\s/.test(prev)) continue;
      }
      // Leave the caret's own token as editable plain text.
      if (caret >= start && caret <= end) continue;
      builder.add(start, end, Decoration.replace({ widget: new FileChipWidget(token) }));
    }
  }
  return builder.finish();
}

export const fileChipPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildChips(view);
    }
    update(u: ViewUpdate): void {
      if (u.docChanged || u.selectionSet || u.viewportChanged) {
        this.decorations = buildChips(u.view);
      }
    }
  },
  {
    decorations: (v) => v.decorations,
    // Atomic so caret motion + backspace treat a chip as one unit.
    provide: (plugin) =>
      EditorView.atomicRanges.of(
        (view) => view.plugin(plugin)?.decorations ?? Decoration.none,
      ),
  },
);
