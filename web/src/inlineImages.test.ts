import { assertEquals } from "jsr:@std/assert";
import {
  imageDeletionRange,
  inlineImageInsertion,
  inlineImagePasteInsertion,
  isImageOnlyLine,
  mapImageDeletionPosition,
} from "./inlineImageSelection";

Deno.test("inline image paste replaces forward or backward selections and lands after the token", () => {
  const expected = {
    from: 6,
    to: 10,
    insert: "\n![shot.png](cowboy-att:image-1)\n \n",
    caret: 40,
  };
  assertEquals(
    inlineImageInsertion(
      "alpha beta gamma",
      6,
      10,
      [{ id: "image-1", name: "shot].png" }],
    ),
    expected,
  );
  assertEquals(
    inlineImageInsertion(
      "alpha beta gamma",
      10,
      6,
      [{ id: "image-1", name: "shot].png" }],
    ),
    expected,
  );
});

Deno.test("a later paste does not replace an already placed image token", () => {
  const first = "![one.png](cowboy-att:image-1)\n ";
  const tokenEnd = first.indexOf(")") + 1;
  const edit = inlineImagePasteInsertion(
    first,
    0,
    first.length,
    [{ id: "image-2", name: "two.png" }],
  );
  assertEquals(edit.from, tokenEnd);
  assertEquals(edit.to, tokenEnd);
  const next = first.slice(0, edit.from) + edit.insert + first.slice(edit.to);
  assertEquals(next.includes("cowboy-att:image-1"), true);
  assertEquals(next.includes("cowboy-att:image-2"), true);
});

Deno.test("a batch paste fills one image row instead of one row per picture", () => {
  const edit = inlineImageInsertion("", 0, 0, [
    { id: "image-1", name: "one.png" },
    { id: "image-2", name: "two.png" },
  ]);
  assertEquals(
    edit.insert,
    "![one.png](cowboy-att:image-1)![two.png](cowboy-att:image-2)\n ",
  );
});

Deno.test("a second paste joins the existing image row and reuses its landing line", () => {
  const first = "![one.png](cowboy-att:image-1)\n ";
  // The caret rests at the end of the landing line after the first paste.
  const edit = inlineImageInsertion(first, first.length, first.length, [
    { id: "image-2", name: "two.png" },
  ]);
  const next = first.slice(0, edit.from) + edit.insert + first.slice(edit.to);
  assertEquals(
    next,
    "![one.png](cowboy-att:image-1)![two.png](cowboy-att:image-2)\n ",
  );
  assertEquals(edit.caret, next.length);
});

Deno.test("an image row still gains a landing line when it has none", () => {
  const row = "![one.png](cowboy-att:image-1)";
  const edit = inlineImageInsertion(row, row.length, row.length, [
    { id: "image-2", name: "two.png" },
  ]);
  const next = row.slice(0, edit.from) + edit.insert + row.slice(edit.to);
  assertEquals(
    next,
    "![one.png](cowboy-att:image-1)![two.png](cowboy-att:image-2)\n ",
  );
  assertEquals(edit.caret, next.length);
});

Deno.test("a paste onto prose still opens its own image row", () => {
  const edit = inlineImageInsertion("notes", 5, 5, [
    { id: "image-1", name: "one.png" },
  ]);
  assertEquals(edit.insert, "\n![one.png](cowboy-att:image-1)\n ");
});

Deno.test("an image row is every line that holds only image tokens", () => {
  assertEquals(isImageOnlyLine("![a](cowboy-att:1)"), true);
  assertEquals(isImageOnlyLine("![a](cowboy-att:1)![b](cowboy-att:2)"), true);
  assertEquals(isImageOnlyLine("look ![a](cowboy-att:1)"), false);
  assertEquals(isImageOnlyLine(" "), false);
});

Deno.test("image deletion removes the insertion line breaks", () => {
  assertEquals(imageDeletionRange(0, 14, 25), { from: 0, to: 15 });
  assertEquals(imageDeletionRange(7, 21, 30), { from: 6, to: 22 });
});

Deno.test("image decorations stay an inline token replace without a presentation branch", async () => {
  const source = await Deno.readTextFile(new URL("./inlineImages.ts", import.meta.url));
  assertEquals(source.includes("block: true"), false);
  assertEquals(source.includes("side: 1"), false);
  assertEquals(source.includes("Decoration.replace({"), true);
  assertEquals(source.includes("atomicRanges"), true);
  assertEquals(source.includes('userSelect: "none"'), true);
  assertEquals(source.includes('widget.contentEditable = "false"'), true);
  assertEquals(source.includes("createInlineImageField"), false);
  assertEquals(source.includes("touchInlineImageField"), false);
  assertEquals(source.includes("tr.reconfigured"), false);
  assertEquals(source.includes("Facet.define"), false);
  assertEquals(source.includes("inlineImagePresentation"), false);
});

Deno.test("reversible image deletion retains the registry entry for undo", async () => {
  const source = await Deno.readTextFile(
    new URL("./inlineImages.ts", import.meta.url),
  );
  const popoverDelete = source.slice(
    source.indexOf("export function removeImageTokenById"),
    source.indexOf("const IMG_BLOCK_RE"),
  );
  const backspaceDelete = source.slice(
    source.indexOf("export function deleteImageTokenBackward"),
    source.indexOf(
      "// Stage 1:",
      source.indexOf("export function deleteImageTokenBackward"),
    ),
  );
  assertEquals(popoverDelete.includes("forgetInlineAttachment"), false);
  assertEquals(backspaceDelete.includes("forgetInlineAttachment"), false);
});

Deno.test("image deletion maps carets before, inside, and after the removed block", () => {
  const from = 6;
  const to = 20;
  assertEquals(mapImageDeletionPosition(5, from, to), 5);
  assertEquals(mapImageDeletionPosition(from, from, to), from);
  assertEquals(mapImageDeletionPosition(12, from, to), from);
  assertEquals(mapImageDeletionPosition(to, from, to), from);
  assertEquals(mapImageDeletionPosition(to + 5, from, to), from + 5);
});
