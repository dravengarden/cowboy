import { assertEquals } from "jsr:@std/assert";
import { EditorState } from "@codemirror/state";
import {
  caretOffImageLineSpec,
  emptyLinePositionsAfterImages,
  selectionOnEmptyLineAfterImage,
  selectionOnEmptyLineInImageChain,
  selectionOnImageLandingLine,
  selectionOnLoneImageLine,
} from "./inlineImageCaretPolicy";

Deno.test("Return on an image line is a normal line break", () => {
  const withTrailer = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n",
    selection: { anchor: 0 },
  });
  assertEquals(selectionOnLoneImageLine(withTrailer), true);
  assertEquals(caretOffImageLineSpec(withTrailer), null);

  const lastLine = EditorState.create({
    doc: "![shot](cowboy-att:image-1)",
    selection: { anchor: 0 },
  });
  assertEquals(caretOffImageLineSpec(lastLine), null);

  const emptyAfter = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n",
    selection: { anchor: 28 },
  });
  assertEquals(selectionOnLoneImageLine(emptyAfter), false);
  assertEquals(caretOffImageLineSpec(emptyAfter), null);
  assertEquals(selectionOnEmptyLineAfterImage(emptyAfter), true);
  assertEquals(selectionOnEmptyLineInImageChain(emptyAfter), true);
  assertEquals(emptyLinePositionsAfterImages(emptyAfter), [28]);

  const laterEmpty = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n\n",
    selection: { anchor: 29 },
  });
  assertEquals(selectionOnEmptyLineAfterImage(laterEmpty), false);
  assertEquals(selectionOnEmptyLineInImageChain(laterEmpty), true);
});

Deno.test("the whitespace landing line under a lone image keeps breaking on Return", () => {
  const token = "![p.png](cowboy-att:p1)";
  const state = (doc: string, at: number): EditorState =>
    EditorState.create({ doc, selection: { anchor: at } });
  assertEquals(selectionOnImageLandingLine(state(`${token}\n `, token.length + 2)), true);
  assertEquals(selectionOnImageLandingLine(state(`${token}\n`, token.length + 1)), true);
  assertEquals(selectionOnImageLandingLine(state(`${token}\n x`, token.length + 2)), false);
  assertEquals(selectionOnImageLandingLine(state(`abc\n `, 5)), false);
  assertEquals(selectionOnImageLandingLine(state(`${token}\n `, token.length)), false);
});
