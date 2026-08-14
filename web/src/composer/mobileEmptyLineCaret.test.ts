import { assertEquals } from "jsr:@std/assert";
import { EditorSelection, EditorState, Transaction } from "@codemirror/state";
import {
  hasDirectDelete,
  hasDirectKeyboardInput,
  isMobileEmptyLineCaretState,
  shouldRepairMobileEmptyLineCaret,
} from "./mobileEmptyLineCaret";

const source = await Deno.readTextFile(
  new URL("./mobileEmptyLineCaret.ts", import.meta.url),
);

Deno.test("mobile caret repair follows a direct input or delete that changes line count", () => {
  assertEquals(
    shouldRepairMobileEmptyLineCaret({
      docChanged: true,
      startLines: 2,
      nextLines: 3,
      directInput: true,
      directDelete: false,
    }),
    true,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret({
      docChanged: true,
      startLines: 3,
      nextLines: 2,
      directInput: false,
      directDelete: true,
    }),
    true,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret({
      docChanged: true,
      startLines: 2,
      nextLines: 3,
      directInput: false,
      directDelete: false,
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
  const del = state.update({
    changes: { from: state.doc.length - 1, to: state.doc.length, insert: "" },
    annotations: Transaction.userEvent.of("delete.backward"),
  });

  assertEquals(hasDirectKeyboardInput([lineBreak]), true);
  assertEquals(hasDirectKeyboardInput([paste]), false);
  assertEquals(hasDirectDelete([del]), true);
  assertEquals(hasDirectDelete([paste]), false);
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

Deno.test("mobile caret anchor stays until the next real input", () => {
  assertEquals(source.includes('anchor.textContent = "\\u200b"'), true);
  assertEquals(source.includes("persistent_until_input"), true);
  assertEquals(source.includes("transaction.docChanged"), true);
  assertEquals(source.includes("touchstart"), false);
  assertEquals(source.includes("pointerdown"), false);
  assertEquals(source.includes("keydown"), false);
  assertEquals(source.includes("flushObservability"), false);
  assertEquals(source.includes("setTimeout"), true);
  assertEquals(source.includes("Never dispatch from `update()`"), true);
  assertEquals(source.includes("setSelectionRange"), false);
  assertEquals(source.includes("drawSelection"), false);
  assertEquals(source.includes(".focus()"), false);
  assertEquals(source.includes("preventDefault"), false);
});
