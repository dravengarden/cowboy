import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  diffNewLineLabels,
  diffPointToNewFile,
  diffSourceProjection,
} from "./diffSourceModel.ts";

const DIFF = [
  "diff --git a/src/main.rs b/src/main.rs",
  "--- a/src/main.rs",
  "+++ b/src/main.rs",
  "@@ -10,3 +20,4 @@ fn main() {",
  " context();",
  "-old_call();",
  "+new_call();",
  "+extra();",
  " }",
].join("\n");

test("diff source projection preserves offsets and hides metadata", () => {
  const projected = diffSourceProjection(DIFF);
  assertEquals(projected.length, DIFF.length);
  assertEquals(
    projected.split("\n")[0],
    " ".repeat(DIFF.split("\n")[0]!.length),
  );
  assertEquals(projected.split("\n")[6], " new_call();");
});

test("diff points map added and context lines to the working tree", () => {
  assertEquals(diffPointToNewFile(DIFF, 4, 4), { row: 19, column: 3 });
  assertEquals(diffPointToNewFile(DIFF, 6, 5), { row: 20, column: 4 });
  assertEquals(diffPointToNewFile(DIFF, 7, 3), { row: 21, column: 2 });
  assertEquals(diffPointToNewFile(DIFF, 8, 2), { row: 22, column: 1 });
});

test("diff points reject deleted lines and metadata", () => {
  assertEquals(diffPointToNewFile(DIFF, 0, 4), null);
  assertEquals(diffPointToNewFile(DIFF, 3, 4), null);
  assertEquals(diffPointToNewFile(DIFF, 5, 4), null);
});

test("diff gutter labels follow new-file lines, not patch rows", () => {
  const { labels, widest } = diffNewLineLabels(
    [
      "diff --git a/a.ts b/a.ts",
      "--- a/a.ts",
      "+++ b/a.ts",
      "@@ -98,4 +98,4 @@ fn",
      " keep",
      "-old",
      "+new",
      " tail",
      "\\ No newline at end of file",
      "",
    ].join("\n"),
  );
  assertEquals(labels, ["", "", "", "", "98", "", "99", "100", "", ""]);
  assertEquals(widest, "100");
});
