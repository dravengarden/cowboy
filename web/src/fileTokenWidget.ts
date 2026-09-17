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
    composingRebuildPending = false;
    constructor(view: EditorView) {
      this.decorations = buildChips(view);
    }
    update(u: ViewUpdate): void {
      // Like Obsidian, never replace widgets under native marked text: pinyin
      // typed right after a chip would otherwise be swallowed into the chip
      // label mid-composition. Map now; rebuild once composition ends.
      if (u.view.composing) {
        if (u.docChanged) this.decorations = this.decorations.map(u.changes);
        this.composingRebuildPending = true;
        return;
      }
      if (
        this.composingRebuildPending || u.docChanged || u.selectionSet ||
        u.viewportChanged
      ) {
        this.composingRebuildPending = false;
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

// A code fence line: ``` / ~~~ (3+) with up to three spaces of indent.
const FENCE_RE = /^\s{0,3}(`{3,}|~{3,})(.*)$/;

interface FenceLine {
  role: "open" | "close";
  /** Line number of the matching fence, or -1 for an unclosed opener. */
  partner: number;
  /** Whether an opener has no info string. */
  bare: boolean;
}

// Pair fences from the top of the document, as CommonMark does: a closer uses
// the opener's character, is at least as long, and has no info string.
function fenceLines(doc: EditorView["state"]["doc"]): Map<number, FenceLine> {
  const lines = new Map<number, FenceLine>();
  let open: { line: number; marker: string } | null = null;
  for (let n = 1; n <= doc.lines; n++) {
    const match = FENCE_RE.exec(doc.line(n).text);
    const marker = match?.[1];
    if (!marker) continue;
    const info = (match[2] ?? "").trim();
    if (!open) {
      open = { line: n, marker };
      lines.set(n, { role: "open", partner: -1, bare: info === "" });
    } else if (
      info === "" && marker[0] === open.marker[0] &&
      marker.length >= open.marker.length
    ) {
      const opener = lines.get(open.line);
      if (opener) opener.partner = n;
      lines.set(n, { role: "close", partner: open.line, bare: true });
      open = null;
    }
  }
  return lines;
}

// Backspace that removes an EMPTY fenced code block as a unit. Typing three
// backticks auto-closes an empty block (rendered as a dark bar); deleting it
// char-by-char leaves an orphaned closing fence. Only the exact empty pair is
// removed: the caret at the END of a bare opening fence, or at the START of
// its closing fence, with only blank lines between. A fence with an info
// string, a closing fence followed by another block, or a block opener that
// follows another block is ordinary text for Backspace — deleting it must
// never merge two blocks or drop code.
export function deleteEmptyCodeFenceBackward(view: EditorView): boolean {
  const { state } = view;
  const range = state.selection.main;
  if (!range.empty) return false;
  const { doc } = state;
  const caret = doc.lineAt(range.head);
  if (!FENCE_RE.test(caret.text)) return false;
  const fence = fenceLines(doc).get(caret.number);
  if (!fence || fence.partner < 0) return false;

  let openLn = -1;
  let closeLn = -1;
  if (fence.role === "open" && fence.bare && range.head === caret.to) {
    openLn = caret.number;
    closeLn = fence.partner;
  } else if (fence.role === "close" && range.head === caret.from) {
    openLn = fence.partner;
    closeLn = caret.number;
  } else {
    return false;
  }
  for (let i = openLn + 1; i < closeLn; i++) {
    if (doc.line(i).text.trim() !== "") return false;
  }

  const from = doc.line(openLn).from;
  const to = Math.min(doc.length, doc.line(closeLn).to + 1);
  view.dispatch({ changes: { from, to }, selection: { anchor: from } });
  return true;
}
