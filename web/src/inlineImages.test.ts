import { assertEquals } from "jsr:@std/assert";
import {
  imageDeletionRange,
  mapImageDeletionPosition,
} from "./inlineImageSelection";

Deno.test("image deletion removes the insertion line breaks", () => {
  assertEquals(imageDeletionRange(0, 14, 25), { from: 0, to: 15 });
  assertEquals(imageDeletionRange(7, 21, 30), { from: 6, to: 22 });
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
