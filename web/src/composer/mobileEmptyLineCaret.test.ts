import { assertEquals } from "jsr:@std/assert";
import { EditorSelection, EditorState, Transaction } from "@codemirror/state";
import {
  hasDirectKeyboardInput,
  isMobileEmptyLineCaretState,
  shouldRepairMobileEmptyLineCaret,
} from "./mobileEmptyLineCaret";

const source = await Deno.readTextFile(
  new URL("./mobileEmptyLineCaret.ts", import.meta.url),
);

Deno.test("mobile caret repair only follows a direct input line addition", () => {
  assertEquals(
    shouldRepairMobileEmptyLineCaret({
      docChanged: true,
      startLines: 2,
      nextLines: 3,
      directInput: true,
    }),
    true,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret({
      docChanged: true,
      startLines: 2,
      nextLines: 3,
      directInput: false,
    }),
    false,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret({
      docChanged: true,
      startLines: 2,
      nextLines: 2,
      directInput: true,
    }),
    false,
  );
});

Deno.test("mobile caret repair excludes paste transactions", () => {
  const state = EditorState.create({ doc: "image\n" });
  const lineBreak = state.update({
    changes: { from: state.doc.length, insert: "\n" },
    annotations: Transaction.userEvent.of("input"),
  });
  const paste = state.update({
    changes: { from: state.doc.length, insert: "\n" },
    annotations: Transaction.userEvent.of("input.paste"),
  });

  assertEquals(hasDirectKeyboardInput([lineBreak]), true);
  assertEquals(hasDirectKeyboardInput([paste]), false);
});

Deno.test("mobile caret repair requires a collapsed empty line", () => {
  const emptyLine = EditorState.create({
    doc: "image\n\n",
    selection: { anchor: 7 },
  });
  assertEquals(isMobileEmptyLineCaretState(emptyLine), true);

  const textLine = EditorState.create({
    doc: "image\ntext",
    selection: { anchor: 10 },
  });
  assertEquals(isMobileEmptyLineCaretState(textLine), false);

  const range = EditorState.create({
    doc: "image\n\n",
    selection: EditorSelection.create([EditorSelection.range(6, 7)]),
  });
  assertEquals(isMobileEmptyLineCaretState(range), false);
});

Deno.test("mobile caret anchor is transient and document neutral", () => {
  assertEquals(source.includes('anchor.textContent = "\\u200b"'), true);
  assertEquals(source.includes("get editable(): boolean"), true);
  assertEquals(source.includes("selection.removeAllRanges()"), true);
  assertEquals(
    source.includes("setMobileEmptyLineCaretAnchor.of(false)"),
    true,
  );
  assertEquals(source.includes("setSelectionRange"), false);
  assertEquals(source.includes("drawSelection"), false);
  assertEquals(source.includes(".focus()"), false);
  assertEquals(source.includes("preventDefault"), false);
  assertEquals(source.includes("changes:"), false);
});
