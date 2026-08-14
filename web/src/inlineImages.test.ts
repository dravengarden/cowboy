import { assertEquals } from "jsr:@std/assert";
import {
  imageDeletionRange,
  inlineImageInsertion,
  mapImageDeletionPosition,
} from "./inlineImageSelection";

Deno.test("inline image paste replaces forward or backward selections and lands after the token", () => {
  const expected = {
    from: 6,
    to: 10,
    insert: "\n![shot.png](cowboy-att:image-1) \n",
    caret: 39,
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

Deno.test("image deletion maps carets before, inside, and after the removed block", () => {
  const from = 6;
  const to = 20;
  assertEquals(mapImageDeletionPosition(5, from, to), 5);
  assertEquals(mapImageDeletionPosition(from, from, to), from);
  assertEquals(mapImageDeletionPosition(12, from, to), from);
  assertEquals(mapImageDeletionPosition(to, from, to), from);
  assertEquals(mapImageDeletionPosition(to + 5, from, to), from + 5);
});
