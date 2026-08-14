import { assertEquals } from "jsr:@std/assert";
import { EditorSelection, EditorState } from "@codemirror/state";
import {
  emptyLinePositionsAfterImages,
  selectionOnEmptyLineAfterImage,
  selectionOnEmptyLineInImageChain,
} from "./inlineImageCaretPolicy";
import {
  collapseLandingSelection,
  isMobileEmptyLineCaretState,
  landingAnchorPositions,
  landingAnchorsForEmptyLinesAfterImages,
  landingSelectionAlreadyPlaced,
  shouldPreventNativeMobileLineBreak,
  updateInsertedLineBreak,
} from "./mobileEmptyLineCaret";

const source = await Deno.readTextFile(
  new URL("./mobileEmptyLineCaret.ts", import.meta.url),
);

Deno.test("image-adjacent empty lines do not mount a landing widget", () => {
  const afterImage = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n",
    selection: { anchor: 28 },
  });
  assertEquals(emptyLinePositionsAfterImages(afterImage), [28]);
  assertEquals(selectionOnEmptyLineAfterImage(afterImage), true);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(afterImage).size, 0);

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
  assertEquals(landingAnchorPositions(laterEmpty), []);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(laterEmpty).size, 0);

  const noImage = EditorState.create({
    doc: "hello\n\n",
    selection: { anchor: 7 },
  });
  assertEquals(emptyLinePositionsAfterImages(noImage), []);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(noImage).size, 0);
});

Deno.test("two images do not expose landing widgets under thumbnails", () => {
  const doc = "![a](cowboy-att:1)\n\n![b](cowboy-att:2)\n";
  const state = EditorState.create({ doc });
  const firstLanding = state.doc.line(2).from;
  const secondLanding = state.doc.line(4).from;
  assertEquals(emptyLinePositionsAfterImages(state), [
    firstLanding,
    secondLanding,
  ]);
  assertEquals(landingAnchorPositions(state), []);
  assertEquals(landingAnchorsForEmptyLinesAfterImages(state).size, 0);
  assertEquals(
    landingAnchorsForEmptyLinesAfterImages(
      EditorState.create({ doc, selection: { anchor: firstLanding } }),
    ).size,
    0,
  );
  assertEquals(
    landingAnchorsForEmptyLinesAfterImages(
      EditorState.create({ doc, selection: { anchor: secondLanding } }),
    ).size,
    0,
  );
});

Deno.test("image-adjacent Return is not intercepted", () => {
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
    shouldPreventNativeMobileLineBreak("insertLineBreak", landing),
    false,
  );
  assertEquals(
    shouldPreventNativeMobileLineBreak("insertParagraph", landing),
    false,
  );
  assertEquals(
    shouldPreventNativeMobileLineBreak("insertLineBreak", laterEmpty),
    false,
  );
  assertEquals(
    shouldPreventNativeMobileLineBreak("insertLineBreak", typed),
    false,
  );
  assertEquals(
    shouldPreventNativeMobileLineBreak("insertText", landing),
    false,
  );
});

Deno.test("landing remap is skipped when the caret is already in the widget", () => {
  const text = { nodeType: 3 } as Node;
  assertEquals(
    landingSelectionAlreadyPlaced(
      { anchorNode: text, isCollapsed: true },
      text,
    ),
    true,
  );
  assertEquals(
    landingSelectionAlreadyPlaced(
      { anchorNode: text, isCollapsed: false },
      text,
    ),
    false,
  );
  assertEquals(landingSelectionAlreadyPlaced(null, text), false);
  assertEquals(typeof updateInsertedLineBreak, "function");

  const moving = {
    anchorNode: null as Node | null,
    isCollapsed: true,
    collapse(node: Node, offset: number) {
      this.anchorNode = node;
      this.isCollapsed = offset === 0;
    },
  };
  assertEquals(
    collapseLandingSelection(moving as unknown as Selection, text),
    true,
  );
  assertEquals(moving.anchorNode, text);
  assertEquals(
    collapseLandingSelection(moving as unknown as Selection, text),
    false,
  );
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
  assertEquals(source.includes("landing node onto the new line"), true);
  assertEquals(source.includes("A U+200B on the"), true);
  assertEquals(source.includes("addEventListener"), true);
  assertEquals(source.includes("onBeforeInputCapture"), true);
  assertEquals(source.includes("other.at === this.at"), true);
  assertEquals(source.includes("userEvent: \"input.type\""), false);
  assertEquals(source.includes("selection.removeAllRanges"), false);
  assertEquals(source.includes("materializeLineBreak"), false);
  assertEquals(source.includes("placeLandingSelection"), true);
  assertEquals(source.includes("touchstart"), false);
  assertEquals(source.includes("pointerdown"), false);
  assertEquals(source.includes("keydown("), false);
  assertEquals(source.includes("flushObservability"), false);
  assertEquals(source.includes("setTimeout"), true);
  assertEquals(source.includes("Never dispatch from update()"), true);
  assertEquals(source.includes("setSelectionRange"), false);
  assertEquals(source.includes("drawSelection"), false);
  assertEquals(source.includes(".focus()"), false);
  assertEquals(source.includes("this.clear()"), false);
});
