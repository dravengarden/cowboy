import { assertEquals } from "jsr:@std/assert";
import {
  applyMarkdownEdit,
  continueMarkdownList,
  cycleHeading,
  type EditorTextSelection,
  indentLines,
  type InlineFormat,
  insertCodeBlock,
  insertMarkdownLink,
  type MarkdownEdit,
  newlineAndIndentOnly,
  outdentLines,
  setHeading,
  toggleBlockquote,
  toggleBulletList,
  toggleChecklist,
  toggleInlineFormat,
  toggleNumberedList,
} from "./markdownEditing";

// Fixture notation: `|` is a collapsed caret; `«` is the anchor and `»` the
// head of a non-empty selection (so `»foo«` is a backward selection). These
// characters never occur in the markdown under test.
function parseFixture(input: string): { doc: string; sel: EditorTextSelection } {
  let doc = "";
  let caret = -1;
  let anchor = -1;
  let head = -1;
  for (const char of input) {
    if (char === "|") caret = doc.length;
    else if (char === "«") anchor = doc.length;
    else if (char === "»") head = doc.length;
    else doc += char;
  }
  if (caret >= 0) return { doc, sel: { anchor: caret, head: caret } };
  if (anchor < 0 || head < 0) throw new Error(`fixture without selection: ${input}`);
  return { doc, sel: { anchor, head } };
}

function renderFixture(doc: string, sel: EditorTextSelection): string {
  if (sel.anchor === sel.head) return doc.slice(0, sel.head) + "|" + doc.slice(sel.head);
  const marks = [
    { pos: sel.anchor, char: "«" },
    { pos: sel.head, char: "»" },
  ].sort((a, b) => b.pos - a.pos);
  let out = doc;
  for (const mark of marks) out = out.slice(0, mark.pos) + mark.char + out.slice(mark.pos);
  return out;
}

type Command = (doc: string, sel: EditorTextSelection) => MarkdownEdit | null;

function run(command: Command, input: string): string | null {
  const { doc, sel } = parseFixture(input);
  const edit = command(doc, sel);
  if (!edit) return null;
  assertSortedChanges(edit, doc.length);
  return renderFixture(applyMarkdownEdit(doc, edit), edit.selection);
}

function assertSortedChanges(edit: MarkdownEdit, length: number): void {
  let pos = 0;
  for (const change of edit.changes) {
    if (change.from < pos || change.to < change.from || change.to > length) {
      throw new Error(`unsorted or overlapping changes: ${JSON.stringify(edit.changes)}`);
    }
    pos = change.to;
  }
}

function table(name: string, command: Command, cases: [string, string | null][]): void {
  Deno.test(name, () => {
    for (const [input, expected] of cases) {
      assertEquals(run(command, input), expected, `input: ${JSON.stringify(input)}`);
    }
  });
}

const inline = (format: InlineFormat): Command => (doc, sel) => toggleInlineFormat(doc, sel, format);

// ---------------------------------------------------------------------------
// Enter / Shift-Enter
// ---------------------------------------------------------------------------

table("Enter continues bullet, task and ordered list items", continueMarkdownList, [
  ["- foo|", "- foo\n- |"],
  ["* foo|", "* foo\n* |"],
  ["- fo|o", "- fo\n- |o"],
  ["- [ ] foo|", "- [ ] foo\n- [ ] |"],
  ["- [x] foo|", "- [x] foo\n- [ ] |"],
  ["- [/] foo|", "- [/] foo\n- [ ] |"],
  ["1. foo|", "1. foo\n2. |"],
  ["9) a|", "9) a\n10) |"],
  ["2. [ ] x|", "2. [ ] x\n3. [ ] |"],
  ["  - nested|", "  - nested\n  - |"],
  ["> - quoted|", "> - quoted\n> - |"],
]);

table("Enter continues quotes and indented continuation lines", continueMarkdownList, [
  ["> foo|", "> foo\n> |"],
  ["  foo|", "  foo\n  |"],
  // A continuation line inherits the list item above with the same prefix width.
  ["- foo\n  bar|", "- foo\n  bar\n- |"],
  ["1. foo\n   bar|", "1. foo\n   bar\n2. |"],
]);

table("Enter on an empty item removes one level of structure", continueMarkdownList, [
  ["- |", "|"],
  ["1. |", "|"],
  ["- [ ] |", "|"],
  ["  - |", "- |"],
  ["\t- |", "- |"],
  ["      - |", "  - |"],
  ["> - |", "> |"],
  // Quote after a non-empty quote line: the marker moves to a fresh line.
  ["> foo\n> |", "> foo\n\n|"],
  // Quote after an already-empty quote line: both markers are removed.
  ["> foo\n> \n> |", "> foo\n\n|"],
  ["> > |", "> \n> |"],
]);

table("Enter falls back to the default newline", continueMarkdownList, [
  ["foo|", null],
  ["|", null],
  ["|- foo", null],
  ["-| foo", null],
  ["- [ ]| foo", null],
  ["«- foo»", null],
]);

table("Enter handles list markers after the caret", continueMarkdownList, [
  // Text after the caret that starts with a marker keeps its own marker.
  ["- foo| - bar", "- foo\n|- bar"],
  // Leading whitespace after the caret is stripped.
  ["- foo| bar", "- foo\n- |bar"],
]);

table("Enter inside a fenced code block only indents", continueMarkdownList, [
  ["```\n- a|\n```", "```\n- a\n  |\n```"],
  ["~~~\n> x|\n~~~", "~~~\n> x\n> |\n~~~"],
  ["```\nplain|\n```", null],
  ["```|\n- a\n```", null],
  ["```\n```\n- a|", "```\n```\n- a\n- |"],
  ["   ```\n- a|", "   ```\n- a\n  |"],
  ["    ```\n- a|", "    ```\n- a\n- |"],
]);

Deno.test("Enter keeps Obsidian's replace-previous-character change shape", () => {
  assertEquals(continueMarkdownList("- foo", { anchor: 5, head: 5 }), {
    changes: [{ from: 4, to: 5, insert: "o\n- " }],
    selection: { anchor: 8, head: 8 },
  });
  assertEquals(continueMarkdownList("- foo - bar", { anchor: 5, head: 5 }), {
    changes: [
      { from: 4, to: 5, insert: "o\n" },
      { from: 5, to: 6, insert: "" },
    ],
    selection: { anchor: 6, head: 6 },
  });
  assertEquals(newlineAndIndentOnly("1. foo", { anchor: 6, head: 6 }), {
    changes: [{ from: 5, to: 6, insert: "o\n   " }],
    selection: { anchor: 10, head: 10 },
  });
});

table("Shift-Enter keeps indentation without a new marker", newlineAndIndentOnly, [
  ["- foo|", "- foo\n  |"],
  ["> foo|", "> foo\n> |"],
  ["1. foo|", "1. foo\n   |"],
  ["- [ ] a|", "- [ ] a\n      |"],
  ["> - a|", "> - a\n>   |"],
  ["  foo|", "  foo\n  |"],
  ["foo|", null],
  ["-| foo", null],
  ["«- foo»", null],
]);

// ---------------------------------------------------------------------------
// Line toggles
// ---------------------------------------------------------------------------

table("bullet list toggle", toggleBulletList, [
  ["fo|o", "- fo|o"],
  ["|foo", "- |foo"],
  ["- foo|", "foo|"],
  ["|- foo", "|foo"],
  // Task and numbered lines count as lacking a bullet, so the box is dropped.
  ["- [ ] foo|", "- foo|"],
  ["2. foo|", "- foo|"],
  ["  - nested|", "  nested|"],
  ["> quote|", "> - quote|"],
  ["«a\n- b\n3. c»", "- «a\n- b\n- c»"],
  ["«- a\n- b»", "«a\nb»"],
  // Blank lines of a multi-line selection are untouched.
  ["«a\n\nb»", "- «a\n\n- b»"],
  ["«a\n  \n- b»", "- «a\n  \n- b»"],
]);

table("numbered list toggle", toggleNumberedList, [
  ["foo|", "1. foo|"],
  ["|foo", "1. |foo"],
  ["1. foo|", "foo|"],
  ["7) foo|", "foo|"],
  ["- [ ] foo|", "1. foo|"],
  ["> foo|", "> 1. foo|"],
  ["«a\n1. b»", "1. «a\n1. b»"],
  ["«1. a\n\n2. b»", "«a\n\nb»"],
]);

table("checklist toggle cycles none → [ ] → [x] → [ ]", toggleChecklist, [
  ["foo|", "- [ ] foo|"],
  ["- foo|", "- [ ] foo|"],
  ["- [ ] foo|", "- [x] foo|"],
  ["- [x] foo|", "- [ ] foo|"],
  ["- [/] foo|", "- [ ] foo|"],
  ["1. foo|", "1. [ ] foo|"],
  ["> foo|", "> - [ ] foo|"],
  ["«- [ ] a\n- [x] b»", "«- [x] a\n- [x] b»"],
  // Obsidian quirk: when some line lacks a box, lines that had one lose it.
  ["«- [x] a\nb»", "«- a\n- [ ] b»"],
]);

table("blockquote toggle", toggleBlockquote, [
  ["foo|", "> foo|"],
  ["|foo", "> |foo"],
  ["> foo|", "foo|"],
  [">foo|", "foo|"],
  ["   > foo|", "foo|"],
  ["> > foo|", "> foo|"],
  ["|", "> |"],
  // Blank lines participate in blockquote toggling.
  ["«a\n\nb»", "> «a\n> \n> b»"],
  ["«> a\nb»", "«> a\n> b»"],
  ["«> a\n> b»", "«a\nb»"],
]);

table("set heading", (doc, sel) => setHeading(doc, sel, 1), [
  ["foo|", "# foo|"],
  ["|foo", "# |foo"],
  ["> ## foo|", "> # foo|"],
  ["### foo|", "# foo|"],
  ["> fo|o", "> # fo|o"],
  // Obsidian: a caret exactly at a pure insertion point stays before it.
  ["> |foo", "> |# foo"],
  ["- foo|", "# - foo|"],
]);

table("set heading levels", (doc, sel) => setHeading(doc, sel, 2), [
  ["«a\n\n# b»", "## «a\n\n## b»"],
]);

table("remove heading", (doc, sel) => setHeading(doc, sel, 0), [
  ["## foo|", "foo|"],
  ["> ###### foo|", "> foo|"],
  ["foo|", "foo|"],
]);

table("heading level clamps to 6", (doc, sel) => setHeading(doc, sel, 9), [
  ["foo|", "###### foo|"],
]);

table("cycle heading none → H1 → H2 → H3 → none", cycleHeading, [
  ["foo|", "# foo|"],
  ["# foo|", "## foo|"],
  ["## foo|", "### foo|"],
  ["### foo|", "foo|"],
  ["###### foo|", "foo|"],
  // The first selected line decides the level for every line.
  ["«# a\nb»", "«## a\n## b»"],
]);

// ---------------------------------------------------------------------------
// Inline formatting
// ---------------------------------------------------------------------------

table("bold toggles on a caret", inline("bold"), [
  ["fo|o", "**fo|o**"],
  ["|foo", "**|foo**"],
  // Obsidian: a caret at the end of the word ends after the closing marker.
  ["foo|", "**foo**|"],
  ["中文|", "**中文**|"],
  ["中|文", "**中|文**"],
  ["|", "**|**"],
  ["a |", "a **|**"],
  ["**bold|**", "**bold**|"],
  ["**bo|ld**", "bo|ld"],
  ["__bo|ld__", "bo|ld"],
  ["x **y|** z", "x **y**| z"],
  // The caret after a closing marker is not "inside" bold, so an empty pair is
  // inserted. Local deviation: the touching closer is kept (Obsidian's literal
  // math removed it and left an unbalanced `**foo**|**`).
  ["**foo**|", "**foo****|**"],
  // Obsidian quirk: caret after the opening marker re-wraps the same word.
  ["**|bold**", "**|bold**"],
]);

table("bold toggles on a selection", inline("bold"), [
  ["«foo»", "**«foo»**"],
  ["»foo«", "**»foo«**"],
  ["«foo »", "**«foo»** "],
  ["« foo »", " **«foo»** "],
  ["**«bold»**", "«bold»"],
  ["«**bold**»", "«bold»"],
  ["x **«y»** z", "x «y» z"],
  ["«x **y** z»", "**«x y z»**"],
  ["«- a\n- b»", "«- **a**\n- **b»**"],
  ["«**a**\n**b**»", "«a\nb»"],
  ["«# foo»", "«# **foo»**"],
  ["# «foo»", "# **«foo»**"],
  ["«- [ ] task»", "«- [ ] **task»**"],
  ["«a\n\nb»", "**«a**\n\n**b»**"],
  ["***«bold»***", "*«bold»*"],
  // Fenced code lines are skipped in a multi-line selection.
  ["«```\na\n```\nb»", "«```\na\n```\n**b»**"],
  ["«   »", "   |"],
]);

table("italic toggles", inline("italic"), [
  // Selecting text inside bold must not count as italic.
  ["**«bold»**", "***«bold»***"],
  ["***«bold»***", "**«bold»**"],
  ["«_it_»", "«it»"],
  ["_«it»_", "«it»"],
  ["*i|t*", "i|t"],
  ["fo|o", "*fo|o*"],
  ["*it|*", "*it*|"],
]);

table("code, highlight and strikethrough toggles", (doc, sel) => {
  const [format, ...rest] = doc.split(":");
  const offset = (format ?? "").length + 1;
  const edit = toggleInlineFormat(
    rest.join(":"),
    { anchor: sel.anchor - offset, head: sel.head - offset },
    format as InlineFormat,
  );
  if (!edit) return null;
  return {
    changes: edit.changes.map((c) => ({ from: c.from + offset, to: c.to + offset, insert: c.insert })),
    selection: { anchor: edit.selection.anchor + offset, head: edit.selection.head + offset },
  };
}, [
  ["code:fo|o", "code:`fo|o`"],
  ["code:`co|de`", "code:co|de"],
  ["code:`«code»`", "code:«code»"],
  ["code:«a b»", "code:`«a b»`"],
  // Obsidian removes one marker width, so a double-backtick span loses one tick.
  ["code:``«x»``", "code:`«x»`"],
  ["highlight:h|i", "highlight:==h|i=="],
  ["highlight:==h|i==", "highlight:h|i"],
  ["highlight:==hi|==", "highlight:==hi==|"],
  ["strikethrough:«x»", "strikethrough:~~«x»~~"],
  ["strikethrough:~~«x»~~", "strikethrough:«x»"],
  ["strikethrough:~~x|~~", "strikethrough:~~x~~|"],
]);

table("math toggles use the same-line text fallback", inline("math"), [
  ["«x»", "$«x»$"],
  ["x|", "$x$|"],
  ["$x|$", "$x$|"],
  ["$x|y$", "x|y"],
  ["a $«x»$ b", "a «x» b"],
  ["$a$ «b» $c$", "$a$ $«b»$ $c$"],
  ["$$«x»$$", "$$$«x»$$$"],
]);

table("comment toggles use the same-line text fallback", inline("comment"), [
  ["«a»", "%%«a»%%"],
  ["%%«a»%%", "«a»"],
  ["%%a|%%", "%%a%%|"],
  ["%%a|b%%", "a|b"],
]);

// ---------------------------------------------------------------------------
// Block insertions
// ---------------------------------------------------------------------------

table("insert code block", insertCodeBlock, [
  ["foo|bar", "```\nfoo|bar\n```"],
  ["|", "```\n|\n```"],
  ["a\n«b\nc»\nd", "a\n```\n«b\nc»\n```\nd"],
  ["a\n»b\nc«\nd", "a\n```\n»b\nc«\n```\nd"],
]);

table("insert markdown link", insertMarkdownLink, [
  ["|", "[|]()"],
  ["a|b", "a[|]()b"],
  ["«sel»", "[sel](|)"],
  ["x »sel« y", "x [sel](|) y"],
]);

// ---------------------------------------------------------------------------
// Indentation
// ---------------------------------------------------------------------------

table("indent lines", (doc, sel) => indentLines(doc, sel, "\t"), [
  ["- a|", "\t- a|"],
  ["|- a", "\t|- a"],
  ["> - a|", "> \t- a|"],
  [">a|", ">\ta|"],
  ["> > a|", "> > \ta|"],
  ["«- a\n- b»", "\t«- a\n\t- b»"],
  // A selection ending at a line start does not indent that line.
  ["«a\n»b", "\t«a\n»b"],
]);

table("indent lines with a space unit", (doc, sel) => indentLines(doc, sel, "  "), [
  ["- a|", "  - a|"],
]);

table("outdent lines", (doc, sel) => outdentLines(doc, sel, "\t"), [
  ["\t- a|", "- a|"],
  ["> \t- a|", "> - a|"],
  ["    - a|", "- a|"],
  ["  - a|", "- a|"],
  ["\t\t- a|", "\t- a|"],
  [">\t\ta|", ">\ta|"],
  ["  > a|", "> a|"],
  ["> a|", null],
  ["a|", null],
  ["«\t- a\n\t\t- b»", "«- a\n\t- b»"],
]);

table("outdent lines with a space unit", (doc, sel) => outdentLines(doc, sel, "  ", 4), [
  ["      - a|", "    - a|"],
  ["\t- a|", "  - a|"],
]);

// ---------------------------------------------------------------------------
// applyMarkdownEdit
// ---------------------------------------------------------------------------

Deno.test("applyMarkdownEdit applies original-coordinate changes", () => {
  const selection = { anchor: 0, head: 0 };
  assertEquals(applyMarkdownEdit("abc", { changes: [], selection }), "abc");
  assertEquals(
    applyMarkdownEdit("hello world", {
      changes: [
        { from: 0, to: 0, insert: "**" },
        { from: 5, to: 5, insert: "**" },
        { from: 6, to: 11, insert: "there" },
      ],
      selection,
    }),
    "**hello** there",
  );
  assertEquals(
    applyMarkdownEdit("ab", {
      changes: [
        { from: 1, to: 1, insert: "X" },
        { from: 1, to: 1, insert: "Y" },
        { from: 1, to: 2, insert: "" },
      ],
      selection,
    }),
    "aXY",
  );
});

Deno.test("returned edits round-trip through applyMarkdownEdit", () => {
  const doc = "- [ ] one\n- two\n\n> three";
  const sel = { anchor: 0, head: doc.length };
  for (const command of [toggleBulletList, toggleNumberedList, toggleChecklist, toggleBlockquote]) {
    const edit = command(doc, sel);
    if (!edit) throw new Error("expected an edit");
    const next = applyMarkdownEdit(doc, edit);
    const back = command(next, edit.selection);
    if (!back) throw new Error("expected a reverse edit");
    assertSortedChanges(back, next.length);
    applyMarkdownEdit(next, back);
  }
  const wrapped = toggleInlineFormat("x **y** z", { anchor: 0, head: 9 }, "bold");
  if (!wrapped) throw new Error("expected an edit");
  const bold = applyMarkdownEdit("x **y** z", wrapped);
  assertEquals(bold, "**x y z**");
  const unwrapped = toggleInlineFormat(bold, wrapped.selection, "bold");
  if (!unwrapped) throw new Error("expected an edit");
  assertEquals(renderFixture(applyMarkdownEdit(bold, unwrapped), unwrapped.selection), "«x y z»");
});
