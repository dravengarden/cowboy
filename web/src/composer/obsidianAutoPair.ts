// Obsidian's typing-time pairing, ported from Obsidian 1.13 app.js.
//
// Obsidian does not use @codemirror/autocomplete's `closeBrackets()` input
// handler for Markdown. It ships a fork with Markdown-specific same-character
// rules, plus a selection-only wrapper for `= ~ $ %`. The upstream handler plus
// the vendored atomic-editor `extendEmphasisPair` produced visible damage in
// Cowboy: typing `**bold**` ended as `**bold**|****`, and `中文**bold**`
// ended with a stray `*`, because upstream pairs a same-character token
// whenever the next character is not a word character.
//
// Obsidian's rules (all guarded by composition state, like upstream):
// - `( [ { ' "`, `* _ \``: typed over a single-line selection wraps it; typed
//   over a multi-line selection replaces it with an empty pair.
// - An opening bracket pairs before whitespace, end of line, or `)]}'":;>`.
// - A same-character token pairs only between whitespace/line edges and never
//   directly after the same token. The character is otherwise literal, so
//   `**` becomes `**|` (the second star steps over the tracked closer) and
//   CJK text before a marker never gets an extra closer.
// - A tracked closer is stepped over; three backticks after `` `` `` that start
//   a node open a fenced block whose closer keeps the list/quote indentation.
// - `= ~ $ %` typed over a selection wrap it and keep it selected.
import {
  type ChangeSpec,
  EditorSelection,
  type EditorState,
  MapMode,
  RangeSet,
  RangeValue,
  type SelectionRange,
  StateEffect,
  StateField,
  type Transaction,
} from "@codemirror/state";
import { syntaxTree } from "@codemirror/language";
import { EditorView, ViewPlugin } from "@codemirror/view";
import { CharCategory, codePointAt, codePointSize } from "@codemirror/state";

const BRACKETS = ["(", "[", "{", "'", '"', "*", "_", "`", "```"];
const CLOSE_BEFORE = ")]}'\":;>";
const SELECTION_WRAP_TOKENS = ["=", "~", "$", "%"];
// Tokens that never collapse a multi-line selection into an empty pair.
const MULTILINE_WRAP_TOKENS = ["%", "`"];
const LIST_PREFIX = /^([>\s]*)(([*+-] |(\d+)([.)] ))(?:\[(.)\] )?)?/;

const android = typeof navigator === "object" &&
  /Android\b/.test(navigator.userAgent);

const closeBracketEffect = StateEffect.define<number>({
  map: (value, mapping) =>
    mapping.mapPos(value, -1, MapMode.TrackAfter) ?? undefined,
});
const skipBracketEffect = StateEffect.define<number>({
  map: (value, mapping) => mapping.mapPos(value),
});

class ClosedBracket extends RangeValue {}
const closedBracket = new ClosedBracket();
closedBracket.startSide = 1;
closedBracket.endSide = -1;

const bracketState = StateField.define<RangeSet<ClosedBracket>>({
  create: () => RangeSet.empty,
  update(value, tr: Transaction) {
    let next = value;
    if (tr.selection) {
      const line = tr.state.doc.lineAt(tr.selection.main.head).from;
      const previous = tr.startState.doc.lineAt(
        tr.startState.selection.main.head,
      ).from;
      if (line !== tr.changes.mapPos(previous, -1)) next = RangeSet.empty;
    }
    next = next.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(closeBracketEffect)) {
        next = next.update({
          add: [closedBracket.range(effect.value, effect.value + 1)],
        });
      } else if (effect.is(skipBracketEffect)) {
        const skipped = effect.value;
        next = next.update({ filter: (from) => from !== skipped });
      }
    }
    return next;
  },
});

function closing(token: string): string {
  const pairs = "()[]{}<>";
  const code = codePointAt(token, 0);
  for (let i = 0; i < pairs.length; i += 2) {
    if (pairs.charCodeAt(i) === code) return pairs.charAt(i + 1);
  }
  return token.charAt(0);
}

function nextChar(state: EditorState, pos: number): string {
  const next = state.doc.sliceString(pos, pos + 2);
  return next.slice(0, codePointSize(codePointAt(next, 0)));
}

function closedBracketAt(state: EditorState, pos: number): boolean {
  let found = false;
  state.field(bracketState).between(0, state.doc.length, (from) => {
    if (from === pos) found = true;
  });
  return found;
}

function spansLines(state: EditorState, range: SelectionRange): boolean {
  return state.doc.lineAt(range.head).number !==
    state.doc.lineAt(range.anchor).number;
}

// Obsidian's "a syntax node starts exactly here" test.
function nodeStart(state: EditorState, pos: number): boolean {
  const node = syntaxTree(state).resolveInner(pos + 1);
  return node.parent !== null && node.from === pos;
}

// Lezer (unlike Obsidian's HyperMD token stream) reports a closing emphasis
// delimiter as its own node, which would make `nodeStart` pair again in front
// of `**bold|**`. Treat a closing delimiter as a closer to step over instead.
// Local deviation, required for the lezer tree.
function closingMarkAt(state: EditorState, pos: number, token: string): boolean {
  const node = syntaxTree(state).resolveInner(pos + 1, -1);
  if (!node.name.endsWith("Mark") || node.from > pos || node.to <= pos) {
    return false;
  }
  const parent = node.parent;
  return parent !== null && parent.to === node.to &&
    parent.from < node.from && state.sliceDoc(pos, pos + 1) === token;
}

// List/quote continuation indent for the closing fence of a new code block.
function fenceIndent(state: EditorState, pos: number): string {
  const match = LIST_PREFIX.exec(state.doc.lineAt(pos).text);
  if (!match) return "";
  return (match[1] ?? "") + " ".repeat((match[3] ?? "").length);
}

function insideFencedCodeBeforeLine(
  state: EditorState,
  lineNumber: number,
): boolean {
  let marker = "";
  let markerLength = 0;
  for (let n = 1; n < lineNumber; n++) {
    const match = /^[>\s]*?(?: {0,3})(`{3,}|~{3,})/.exec(state.doc.line(n).text);
    const fence = match?.[1];
    if (!fence) continue;
    if (!marker) {
      marker = fence.charAt(0);
      markerLength = fence.length;
    } else if (fence.charAt(0) === marker && fence.length >= markerLength) {
      marker = "";
      markerLength = 0;
    }
  }
  return marker !== "";
}

function update(
  state: EditorState,
  spec: ReturnType<EditorState["changeByRange"]>,
): Transaction {
  return state.update(spec, { scrollIntoView: true, userEvent: "input.type" });
}

function handleOpen(
  state: EditorState,
  open: string,
  close: string,
): Transaction | null {
  let refused = false;
  const spec = state.changeByRange((range) => {
    if (!range.empty) {
      if (spansLines(state, range)) {
        return {
          changes: [{ insert: open + close, from: range.from, to: range.to }],
          effects: closeBracketEffect.of(range.from + open.length),
          range: EditorSelection.cursor(range.from + open.length),
        };
      }
      return {
        changes: [
          { insert: open, from: range.from },
          { insert: close, from: range.to },
        ],
        effects: closeBracketEffect.of(range.to + open.length),
        range: EditorSelection.range(
          range.anchor + open.length,
          range.head + open.length,
        ),
      };
    }
    const next = nextChar(state, range.head);
    if (!next || /\s/.test(next) || CLOSE_BEFORE.includes(next)) {
      return {
        changes: { insert: open + close, from: range.head },
        effects: closeBracketEffect.of(range.head + open.length),
        range: EditorSelection.cursor(range.head + open.length),
      };
    }
    refused = true;
    return { range };
  });
  return refused ? null : update(state, spec);
}

function handleClose(state: EditorState, close: string): Transaction | null {
  let refused = false;
  const ranges = state.selection.ranges.map((range) => {
    if (range.empty && nextChar(state, range.head) === close) {
      return EditorSelection.cursor(range.head + close.length);
    }
    refused = true;
    return range;
  });
  if (refused) return null;
  return state.update({
    selection: EditorSelection.create(ranges, state.selection.mainIndex),
    scrollIntoView: true,
    effects: state.selection.ranges.map((range) =>
      skipBracketEffect.of(range.from)
    ),
  });
}

function handleSame(
  state: EditorState,
  token: string,
  allowTriple: boolean,
): Transaction | null {
  let refused = false;
  const spec = state.changeByRange((range) => {
    if (!range.empty) {
      let open = token;
      let close = token;
      if (
        allowTriple && range.from >= 2 &&
        state.sliceDoc(range.from - 2, range.from) === token + token
      ) {
        const indent = fenceIndent(state, range.from);
        if (state.sliceDoc(range.from, range.from + 1) !== "\n") {
          open = `${token}\n${indent}`;
        }
        if (state.sliceDoc(range.to - 1, range.to) !== "\n") {
          close = `\n${indent}${token}`;
        }
      } else if (
        !MULTILINE_WRAP_TOKENS.includes(token) && spansLines(state, range)
      ) {
        return {
          changes: [{ insert: open + close, from: range.from, to: range.to }],
          effects: closeBracketEffect.of(range.from + open.length),
          range: EditorSelection.cursor(range.from + open.length),
        };
      }
      return {
        changes: [
          { insert: open, from: range.from },
          { insert: close, from: range.to },
        ],
        effects: closeBracketEffect.of(range.to + open.length),
        range: EditorSelection.range(
          range.anchor + open.length,
          range.head + open.length,
        ),
      };
    }

    const pos = range.head;
    const next = nextChar(state, pos);
    if (next === token) {
      if (closedBracketAt(state, pos) || closingMarkAt(state, pos, token)) {
        const triple = allowTriple &&
          state.sliceDoc(pos, pos + 3 * token.length) === token.repeat(3);
        return {
          changes: [],
          effects: skipBracketEffect.of(pos),
          range: EditorSelection.cursor(pos + token.length * (triple ? 3 : 1)),
        };
      }
      if (nodeStart(state, pos)) {
        return {
          changes: { insert: token + token, from: pos },
          effects: closeBracketEffect.of(pos + token.length),
          range: EditorSelection.cursor(pos + token.length),
        };
      }
    } else if (
      allowTriple &&
      state.sliceDoc(pos - 2 * token.length, pos) === token + token &&
      nodeStart(state, pos - 2 * token.length) &&
      !insideFencedCodeBeforeLine(state, state.doc.lineAt(pos).number)
    ) {
      const insert = `${token}\n${fenceIndent(state, range.from)}${
        token.repeat(3)
      }`;
      return {
        changes: { insert, from: pos },
        effects: closeBracketEffect.of(pos + token.length),
        range: EditorSelection.cursor(pos + token.length),
      };
    } else if (state.charCategorizer(pos)(next) === CharCategory.Space) {
      const before = state.sliceDoc(pos - 1, pos);
      if (
        before !== token &&
        state.charCategorizer(pos)(before) === CharCategory.Space
      ) {
        return {
          changes: { insert: token + token, from: pos },
          effects: closeBracketEffect.of(pos + token.length),
          range: EditorSelection.cursor(pos + token.length),
        };
      }
    }
    refused = true;
    return { range };
  });
  return refused ? null : update(state, spec);
}

/** Obsidian's pairing decision for one typed string, or null for native input. */
export function obsidianInsertBracket(
  state: EditorState,
  typed: string,
): Transaction | null {
  for (const token of BRACKETS) {
    const close = closing(token);
    if (typed === token) {
      return close === token
        ? handleSame(state, token, BRACKETS.includes(token.repeat(3)))
        : handleOpen(state, token, close);
    }
    if (
      typed === close && close !== token &&
      closedBracketAt(state, state.selection.main.from)
    ) {
      return handleClose(state, close);
    }
  }
  return null;
}

function acceptsTypedToken(
  view: EditorView,
  from: number,
  to: number,
  text: string,
): boolean {
  const main = view.state.selection.main;
  return !(text.length > 2 ||
    (text.length === 2 && codePointSize(codePointAt(text, 0)) === 1) ||
    from !== main.from || to !== main.to);
}

const pairInputHandler = EditorView.inputHandler.of(
  (view, from, to, text) => {
    if (
      (android ? view.composing : view.compositionStarted) ||
      view.state.readOnly || !acceptsTypedToken(view, from, to, text)
    ) {
      return false;
    }
    const tr = obsidianInsertBracket(view.state, text);
    if (!tr) return false;
    view.dispatch(tr);
    return true;
  },
);

/** Obsidian's `= ~ $ %` wrap: only acts on a non-empty selection. */
export function obsidianWrapSelection(
  state: EditorState,
  typed: string,
): Transaction | null {
  if (!SELECTION_WRAP_TOKENS.includes(typed)) return null;
  let refused = false;
  const spec = state.changeByRange((range) => {
    if (range.empty) {
      refused = true;
      return { range };
    }
    const changes: ChangeSpec[] = [
      { insert: typed, from: range.from },
      { insert: typed, from: range.to },
    ];
    return {
      changes,
      range: EditorSelection.range(
        range.anchor + typed.length,
        range.head + typed.length,
      ),
    };
  });
  return refused ? null : update(state, spec);
}

const selectionWrapInputHandler = EditorView.inputHandler.of(
  (view, from, to, text) => {
    if (
      view.composing || view.state.readOnly ||
      !acceptsTypedToken(view, from, to, text)
    ) {
      return false;
    }
    const tr = obsidianWrapSelection(view.state, text);
    if (!tr) return false;
    view.dispatch(tr);
    return true;
  },
);

/** Replaces `closeBrackets()` + `extendEmphasisPair` + `autoCloseCodeFence`. */
export const obsidianAutoPair = [
  pairInputHandler,
  bracketState,
  selectionWrapInputHandler,
];

// iOS smart punctuation turns a second `-` at the start of a line into an em
// dash, which breaks `--` / `---` Markdown. Obsidian's iOS app restores the two
// hyphens; keep the same line-start-only rule.
const lastInsertPlugin = ViewPlugin.define(() => ({ lastInsert: "" }));

export const iosLineStartDashRepair = [
  lastInsertPlugin,
  EditorView.inputHandler.of((view, from, to, text) => {
    if (view.compositionStarted || view.state.readOnly || from !== to) {
      return false;
    }
    const tracker = view.plugin(lastInsertPlugin);
    if (!tracker) return false;
    const restore = tracker.lastInsert === "-" && text === "—" &&
      view.state.doc.lineAt(from).from === from;
    tracker.lastInsert = text;
    if (!restore) return false;
    view.dispatch({
      changes: [{ insert: "--", from }],
      selection: { anchor: from + 2 },
      userEvent: "input.type",
    });
    return true;
  }),
];
