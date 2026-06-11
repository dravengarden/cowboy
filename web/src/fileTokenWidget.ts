import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { RangeSetBuilder } from "@codemirror/state";

// Render completed `@path` references AND `/skill` commands as Material chips —
// one consistent look for both. Display-only: the underlying doc text stays
// `@path` / `/skill`, so the prompt sent to the agent is unchanged.
//
// A token is chipped only once it's *completed* — i.e. immediately followed by
// whitespace (the autocomplete `apply` inserts a trailing space). The token
// being actively typed (no trailing space yet) stays plain text so the picker
// and editing work. Chips are atomic for caret motion; whole-token delete is
// handled by `deleteTokenBackward` below (a trailing-space-aware Backspace).
class TokenChipWidget extends WidgetType {
  constructor(private readonly label: string) {
    super();
  }
  override eq(other: TokenChipWidget): boolean {
    return other.label === this.label;
  }
  override toDOM(): HTMLElement {
    const span = document.createElement("span");
    span.className = "cm-token-chip";
    span.textContent = this.label;
    return span;
  }
  override ignoreEvent(): boolean {
    return false;
  }
}

// `@token` at start-or-after-whitespace, OR a leading `/token`. Each must be
// followed by whitespace to count as completed.
const AT_RE = /(?:^|\s)(@\S+)(?=\s)/g;
const SLASH_RE = /^(\/\S+)(?=\s)/;

function buildChips(view: EditorView): DecorationSet {
  const doc = view.state.doc;
  const ranges: { from: number; to: number; token: string }[] = [];

  // Leading `/skill` (slash commands are first-position only).
  const head = doc.sliceString(0, Math.min(doc.length, 300));
  const sm = SLASH_RE.exec(head);
  if (sm?.[1]) ranges.push({ from: 0, to: sm[1].length, token: sm[1] });

  // `@path` references in the visible ranges.
  for (const { from, to } of view.visibleRanges) {
    const text = doc.sliceString(from, to);
    AT_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = AT_RE.exec(text)) !== null) {
      const token = m[1];
      if (token === undefined) continue;
      const start = from + m.index + (m[0].length - token.length);
      const end = start + token.length;
      if (start > 0) {
        const prev = doc.sliceString(start - 1, start);
        if (prev && !/\s/.test(prev)) continue; // mid-word `@` (e.g. email)
      }
      ranges.push({ from: start, to: end, token });
    }
  }

  ranges.sort((a, b) => a.from - b.from);
  const builder = new RangeSetBuilder<Decoration>();
  for (const r of ranges) {
    builder.add(r.from, r.to, Decoration.replace({ widget: new TokenChipWidget(r.token) }));
  }
  return builder.finish();
}

export const tokenChipPlugin = ViewPlugin.fromClass(
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
    // Atomic so arrow keys jump over a chip as one unit.
    provide: (plugin) =>
      EditorView.atomicRanges.of(
        (view) => view.plugin(plugin)?.decorations ?? Decoration.none,
      ),
  },
);

// Backspace that removes a whole completed token in one press. Handles the
// common case (caret just after `@path ` / `/skill ` — token + its trailing
// space) and the caret-right-before-the-space case. Returns false (→ normal
// backspace) while a token is being typed, so editing still works char-by-char.
export function deleteTokenBackward(view: EditorView): boolean {
  const { state } = view;
  const range = state.selection.main;
  if (!range.empty) return false;
  const head = range.head;
  const before = state.doc.sliceString(Math.max(0, head - 300), head);

  const isLeadingSlash = (tokenStart: number, token: string): boolean =>
    !token.startsWith("/") || tokenStart === 0;

  // (a) caret after "token " — delete token + the trailing space together.
  let m = /(?:^|\s)([@/]\S+) $/.exec(before);
  if (m?.[1]) {
    const from = head - m[1].length - 1;
    if (isLeadingSlash(from, m[1])) {
      view.dispatch({ changes: { from, to: head }, selection: { anchor: from } });
      return true;
    }
  }
  // (b) caret right after a token whose next char is whitespace (chip edge).
  const after = state.doc.sliceString(head, head + 1);
  if (after && /\s/.test(after)) {
    m = /(?:^|\s)([@/]\S+)$/.exec(before);
    if (m?.[1]) {
      const from = head - m[1].length;
      if (isLeadingSlash(from, m[1])) {
        view.dispatch({ changes: { from, to: head }, selection: { anchor: from } });
        return true;
      }
    }
  }
  return false;
}

// A line that is (just) a code fence: ``` / ~~~ (3+), optional indent + info.
const FENCE_RE = /^\s{0,3}(`{3,}|~{3,})/;

// Backspace that removes an EMPTY fenced code block as a unit. `autoCloseCodeFence`
// turns a typed ``` into ```\n``` (an empty block, rendered as a dark bar); deleting
// it char-by-char leaves an orphaned closing fence — the "往回删除还会留一小段"
// leftover. When the caret sits at the END of the opening fence (or the START of the
// closing fence) of an empty block, delete the whole block in one press. Returns
// false otherwise, so normal Backspace / token-delete still run. Wire it in the
// Backspace keymap alongside the token deleters.
export function deleteEmptyCodeFenceBackward(view: EditorView): boolean {
  const { state } = view;
  const range = state.selection.main;
  if (!range.empty) return false;
  const { doc } = state;
  const caret = doc.lineAt(range.head);
  if (!FENCE_RE.test(caret.text)) return false;

  let openLn = -1;
  let closeLn = -1;
  if (range.head === caret.to) {
    // Caret at end of an OPENING fence → the block runs down to the next fence,
    // with only blank lines (an empty body) between.
    openLn = caret.number;
    for (let i = caret.number + 1; i <= doc.lines; i++) {
      const t = doc.line(i).text;
      if (FENCE_RE.test(t)) { closeLn = i; break; }
      if (t.trim() !== "") return false; // real content → leave it alone
    }
  } else if (range.head === caret.from) {
    // Caret at start of a CLOSING fence → the block runs up to the previous fence.
    closeLn = caret.number;
    for (let i = caret.number - 1; i >= 1; i--) {
      const t = doc.line(i).text;
      if (FENCE_RE.test(t)) { openLn = i; break; }
      if (t.trim() !== "") return false;
    }
  } else {
    return false; // mid-fence (e.g. editing the info string) → don't touch
  }
  if (openLn === -1 || closeLn === -1 || closeLn <= openLn) return false;

  const from = doc.line(openLn).from;
  const to = Math.min(doc.length, doc.line(closeLn).to + 1);
  view.dispatch({ changes: { from, to }, selection: { anchor: from } });
  return true;
}
