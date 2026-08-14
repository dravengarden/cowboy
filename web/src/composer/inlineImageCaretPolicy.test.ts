import { assertEquals } from "jsr:@std/assert";
import { EditorState } from "@codemirror/state";
import {
  caretOffImageLineSpec,
  selectionOnLoneImageLine,
} from "./inlineImageCaretPolicy";

Deno.test("Return on an image line moves to the trailing empty line", () => {
  const withTrailer = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n",
    selection: { anchor: 0 },
  });
  assertEquals(selectionOnLoneImageLine(withTrailer), true);
  assertEquals(caretOffImageLineSpec(withTrailer), { selection: { anchor: 28 } });

  const lastLine = EditorState.create({
    doc: "![shot](cowboy-att:image-1)",
    selection: { anchor: 0 },
  });
  assertEquals(caretOffImageLineSpec(lastLine), {
    changes: { from: 27, insert: "\n" },
    selection: { anchor: 28 },
  });

  const emptyAfter = EditorState.create({
    doc: "![shot](cowboy-att:image-1)\n",
    selection: { anchor: 28 },
  });
  assertEquals(selectionOnLoneImageLine(emptyAfter), false);
  assertEquals(caretOffImageLineSpec(emptyAfter), null);
});
