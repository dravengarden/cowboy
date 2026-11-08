import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { createPatchDecoder, decodePatch } from "./unifiedPatchDecoder.ts";
import { splitDiff, splitRowOf } from "./splitDiffModel.ts";

const PATCH = [
  "diff --git a/x.ts b/x.ts",
  "--- a/x.ts",
  "+++ b/x.ts",
  "@@ -1,4 +1,4 @@",
  " keep",
  "-old one",
  "-old two",
  "+new one",
  " tail",
  "\\ No newline at end of file",
  "@@ -10,2 +10,3 @@",
  " ten",
  "+added",
  "+added again",
  "",
].join("\n");

test("decoding is independent of where the pages were cut", () => {
  const whole = decodePatch(PATCH);
  for (let cut = 0; cut <= PATCH.length; cut++) {
    for (const second of [cut, Math.min(PATCH.length, cut + 7)]) {
      const decoder = createPatchDecoder();
      decoder.push(PATCH.slice(0, cut));
      decoder.push(PATCH.slice(cut, second));
      decoder.push(PATCH.slice(second));
      assertEquals(decoder.finish(), whole);
    }
  }
});

test("hunks carry file lines, no-newline marks and patch rows", () => {
  const patch = decodePatch(PATCH);
  assertEquals(patch.hunks.length, 2);
  const [first, second] = patch.hunks;
  assertEquals(first!.lines.map((line) => [line.kind, line.oldLine, line.newLine]), [
    ["context", 1, 1], ["delete", 2, undefined], ["delete", 3, undefined], ["add", undefined, 2], ["context", 4, 3],
  ]);
  assertEquals(first!.lines.at(-1)!.noNewline, true);
  assertEquals(second!.lines.map((line) => line.row), [12, 13, 14]);
});

test("CRLF, renames and binary rows are metadata, not hunk text", () => {
  const crlf = decodePatch("@@ -1 +1 @@\r\n-a\r\n+b\r\n");
  assertEquals([crlf.crlf, crlf.hunks[0]!.lines.map((line) => line.text)], [true, ["a", "b"]]);
  const rename = decodePatch("diff --git a/a b/b\nsimilarity index 100%\nrename from a\nrename to b\n");
  assertEquals([rename.renameFrom, rename.renameTo, rename.hunks.length], ["a", "b", 0]);
  assertEquals(decodePatch("diff --git a/i.png b/i.png\nBinary files a/i.png and b/i.png differ\n").binary, true);
  assertEquals(splitDiff(rename).right.text, "renamed to b");
});

test("the split aligns deletions with additions and pads the shorter side", () => {
  const split = splitDiff(decodePatch(PATCH));
  assertEquals(split.left.kinds, ["hunk", "context", "delete", "delete", "context", "hunk", "context", "filler", "filler"]);
  assertEquals(split.right.kinds, ["hunk", "context", "add", "filler", "context", "hunk", "context", "add", "add"]);
  assertEquals(split.left.labels.slice(1, 5), ["1", "2", "3", "4"]);
  assertEquals(split.right.labels.slice(1, 5), ["1", "2", "", "3"]);
  assertEquals(split.left.text.split("\n").length, split.right.text.split("\n").length);
  assertEquals(split.unifiedRows, [4, 5, 8, 7, 9, 11, 12, 13, 14]);
  assertEquals(splitRowOf(split, 13), 8);
  assertEquals(splitRowOf(split, 10), 5);
});
