import { assertEquals } from "jsr:@std/assert";
import { EditorSelection, EditorState } from "@codemirror/state";
import {
  emptyLinePositionsAfterImages,
  selectionOnEmptyLineAfterImage,
  selectionOnEmptyLineInImageChain,
} from "./inlineImageCaretPolicy";
import {
  isMobileEmptyLineCaretState,
  landingAnchorsForEmptyLinesAfterImages,
  landingLineBreakSpec,
  shouldMaterializeMobileEmptyLineBreak,
} from "./mobileEmptyLineCaret";

const source = await Deno.readTextFile(
  new URL("./mobileEmptyLineCaret.ts", import.meta.url),
);

Deno.test("landing anchors sit only on empty lines after images", () => {
  const afterImage = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n",
    selection: { anchor: 28 },
  });
  assertEquals(emptyLinePositionsAfterImages(afterImage), [28]);
  assertEquals(selectionOnEmptyLineAfterImage(afterImage), true);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(afterImage).size, 1);

  const typed = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\nhello",
    selection: { anchor: 33 },
  });
  assertEquals(emptyLinePositionsAfterImages(typed), []);
  assertEquals(selectionOnEmptyLineAfterImage(typed), false);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(typed).size, 0);

  const laterEmpty = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n\n",
    selection: { anchor: 29 },
  });
  assertEquals(emptyLinePositionsAfterImages(laterEmpty), [28]);
  assertEquals(selectionOnEmptyLineAfterImage(laterEmpty), false);
  assertEquals(selectionOnEmptyLineInImageChain(laterEmpty), true);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(laterEmpty).size, 1);

  const noImage = EditorState.create({
    doc: "hello\n\n",
    selection: { anchor: 7 },
  });
  assertEquals(emptyLinePositionsAfterImages(noImage), []);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(noImage).size, 0);
});

Deno.test("two images expose a landing position under each thumbnail", () => {
  const doc = "![a](cowboy-att:1)\n\n![b](cowboy-att:2)\n";
  const state = EditorState.create({ doc });
  const firstLanding = state.doc.line(2).from;
  const secondLanding = state.doc.line(4).from;
  assertEquals(emptyLinePositionsAfterImages(state), [
    firstLanding,
    secondLanding,
  ]);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(state).size, 0);
  assertEquals(
    landingAnchorsForEmptyLinesAfterImages(
      EditorState.create({ doc, selection: { anchor: firstLanding } }),
    ).size,
    1,
  );
  assertEquals(
    landingAnchorsForEmptyLinesAfterImages(
      EditorState.create({ doc, selection: { anchor: secondLanding } }),
    ).size,
    1,
  );
});

Deno.test("Return inside a landing anchor becomes a document line break", () => {
  const landing = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n",
    selection: { anchor: 28 },
  });
  const laterEmpty = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n\n",
    selection: { anchor: 29 },
  });
  const typed = EditorState.create({
    doc: "hello\n\n",
    selection: { anchor: 7 },
  });
  assertEquals(
    shouldMaterializeMobileEmptyLineBreak("insertLineBreak", landing),
    true,
  );
  assertEquals(
    shouldMaterializeMobileEmptyLineBreak("insertParagraph", landing),
    true,
  );
  assertEquals(
    shouldMaterializeMobileEmptyLineBreak("insertLineBreak", laterEmpty),
    true,
  );
  assertEquals(
    shouldMaterializeMobileEmptyLineBreak("insertLineBreak", typed),
    false,
  );
  assertEquals(
    shouldMaterializeMobileEmptyLineBreak("insertText", landing),
    false,
  );

  assertEquals(landingLineBreakSpec(landing), {
    from: 28,
    insert: "\n",
    anchor: 29,
  });
  assertEquals(landingLineBreakSpec(laterEmpty), {
    from: 29,
    insert: "\n",
    anchor: 30,
  });
  assertEquals(landingLineBreakSpec(typed), null);
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

Deno.test("mobile caret landing anchor is document-neutral and not late-mounted", () => {
  assertEquals(source.includes('anchor.textContent = "\\u200b"'), true);
  assertEquals(source.includes("selectionOnEmptyLineInImageChain"), true);
  assertEquals(source.includes("beforeinput.target is always .cm-content"), true);
  assertEquals(source.includes("placeLandingSelection"), true);
  assertEquals(source.includes("touchstart"), false);
  assertEquals(source.includes("pointerdown"), false);
  assertEquals(source.includes("keydown"), false);
  assertEquals(source.includes("flushObservability"), false);
  assertEquals(source.includes("setTimeout"), true);
  assertEquals(source.includes("Never dispatch from update()"), true);
  assertEquals(source.includes("setSelectionRange"), false);
  assertEquals(source.includes("drawSelection"), false);
  assertEquals(source.includes(".focus()"), false);
  assertEquals(source.includes("this.clear()"), false);
});
