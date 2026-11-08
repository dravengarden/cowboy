import { expect, test } from "bun:test";
import { diffBaseOid, expandDiffContext } from "./diffContextExpansion";

const HEADER = "diff --git a/f.txt b/f.txt\nindex 1111111..2222222 100644\n--- a/f.txt\n+++ b/f.txt\n";
const BASE = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";

test("the base oid comes from the index line of a single text file", () => {
  expect(diffBaseOid(`${HEADER}@@ -1 +1 @@\n-1\n+one\n`)).toBe("1111111");
  expect(diffBaseOid("diff --git a/n b/n\nnew file mode 100644\nindex 0000000..2222222\n")).toBeUndefined();
  expect(diffBaseOid(`${HEADER}${HEADER}`)).toBeUndefined();
  expect(diffBaseOid("diff --git a/b b/b\nindex 1111111..2222222\nBinary files a/b and b/b differ\n")).toBe("1111111");
});

test("gaps fill from the base so the whole file reads as one patch", () => {
  const diff = `${HEADER}@@ -2,2 +2,2 @@ label\n 2\n-3\n+three\n@@ -7,2 +7,3 @@\n 7\n+7.5\n 8\n`;
  expect(expandDiffContext(diff, BASE)).toBe(
    `${HEADER}@@ -1,6 +1,6 @@ label\n 1\n 2\n-3\n+three\n 4\n 5\n 6\n@@ -7,4 +7,5 @@\n 7\n+7.5\n 8\n 9\n 10\n`,
  );
});

test("a pure insertion and a missing final newline keep their positions", () => {
  const base = "a\nb\nc";
  const diff = `${HEADER}@@ -1,0 +2 @@\n+inserted\n`;
  expect(expandDiffContext(diff, base)).toBe(
    `${HEADER}@@ -1,3 +1,4 @@\n a\n+inserted\n b\n c\n\\ No newline at end of file\n`,
  );
});

test("a patch that disagrees with the base, or no hunk, is refused", () => {
  expect(expandDiffContext(`${HEADER}@@ -2,1 +2,1 @@\n-not-two\n+x\n`, BASE)).toBeUndefined();
  expect(expandDiffContext(`${HEADER}@@ -2,2 +2,1 @@\n-2\n+x\n`, BASE)).toBeUndefined();
  expect(expandDiffContext(HEADER, BASE)).toBeUndefined();
});

test("CRLF lines compare with their carriage returns", () => {
  const base = "a\r\nb\r\nc\r\n";
  const diff = `${HEADER}@@ -2 +2 @@\n-b\r\n+B\r\n`;
  expect(expandDiffContext(diff, base)).toBe(`${HEADER}@@ -1,3 +1,3 @@\n a\r\n-b\r\n+B\r\n c\r\n`);
});
