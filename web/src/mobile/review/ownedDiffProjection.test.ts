import { assert, assertEquals } from "jsr:@std/assert";
import {
  projectedDiffPoint,
  projectReviewDiff,
  type ReviewDiffProjection,
} from "./ownedDiffProjection.ts";
import { reviewDisplayText } from "./reviewDisplayText.ts";

const patch =
  "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1,2 +1,3 @@\n first\n-old\n+a🙂z\n+last\n";
const source = "first\na🙂z\nlast\n";

Deno.test("owned diff captures complete source and maps only matching new-side UTF-16", () => {
  const proof = projectReviewDiff(patch, source);
  assert(proof);
  assertEquals(proof.source, source);
  assertEquals(proof.patch, patch);
  assert(Object.isFrozen(proof));
  assertEquals(projectedDiffPoint(proof, 4, 2), { row: 0, column: 1 });
  assertEquals(projectedDiffPoint(proof, 6, 4), { row: 1, column: 3 });
  assertEquals(projectedDiffPoint(proof, 7, 5), { row: 2, column: 4 });
});
Deno.test("owned diff cannot map old lines, metadata, out-of-range or split surrogates", () => {
  const proof = projectReviewDiff(patch, source)!;
  for (
    const [row, column] of [
      [0, 3],
      [3, 2],
      [5, 2],
      [8, 1],
      [6, 0],
      [6, 3],
      [6, 6],
      [-1, 1],
      [6.5, 1],
      [6, NaN],
      [Infinity, 1],
    ]
  ) {
    assertEquals(projectedDiffPoint(proof, row!, column!), null);
  }
  assertEquals(
    projectedDiffPoint({ ...proof } as ReviewDiffProjection, 6, 4),
    null,
  );
  assertEquals(
    // @ts-expect-error a structural JSON record is not a local projection
    projectedDiffPoint({ patch, source }, 6, 4),
    null,
  );
});
Deno.test("a stale untouched new-side row refuses the entire patch, not just that row", () => {
  assertEquals(
    projectReviewDiff(patch, source.replace("last", "else")),
    undefined,
  );
  assertEquals(projectReviewDiff(patch, `extra\n${source}`), undefined);
  assertEquals(
    projectReviewDiff(patch, source.replace("a🙂z", "a😃z")),
    undefined,
  );
});
Deno.test("multiple separated hunks capture hidden current text without claiming old Git identity", () => {
  const patch =
    "@@ -1 +1 @@\n-old\n+first\n@@ -3 +3 @@ function\n-end\n+last\n";
  const proof = projectReviewDiff(
    patch,
    "first\nhidden current content\nlast\n",
  );
  assert(proof);
  assertEquals(projectedDiffPoint(proof, 5, 2), { row: 2, column: 1 });
});
Deno.test("new files, deletions and hunk suffixes that resemble metadata are parsed by counts", () => {
  assert(
    projectReviewDiff(
      "@@ -0,0 +1,2 @@\n+++value\n+---value\n",
      "++value\n---value\n",
    ),
  );
  assert(projectReviewDiff("@@ -1,2 +1 @@\n-old\n same\n", "same\n"));
  assertEquals(projectReviewDiff("@@ -1 +0,0 @@\n-old\n", ""), undefined);
});
Deno.test("malformed, overlapping, excess and incomplete hunk ranges cannot authorize positions", () => {
  for (
    const invalid of [
      patch.replace("-1,2", "-1,3"),
      patch.replace("+1,3", "+1,4"),
      patch.replace("+1,3", "+1,2"),
      patch.replace("+1,3", "+0,3"),
      patch.replace("+1,3", "+4294967296,3"),
      patch.replace("-1,2", "-0,2"),
      `${patch}@@ -1 +1 @@\n first\n`,
      `${patch}+extra\n`,
      `${patch}diff --git a/other b/other\n`,
      patch.replace("@@ -", "@@@ -"),
      patch.replace("+1,3", "+1,NaN"),
      patch.replace("@@ -1,2 +1,3 @@", "@@ -0,0 +0,0 @@"),
    ]
  ) assertEquals(projectReviewDiff(invalid, source), undefined, invalid);
});
Deno.test("EOF newline identity is required and old-side markers never certify new content", () => {
  const noNewline = "@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file\n";
  assert(projectReviewDiff(noNewline, "new"));
  assertEquals(projectReviewDiff(noNewline, "new\n"), undefined);
  assertEquals(
    projectReviewDiff(
      noNewline.replace("\\ No newline at end of file\n", ""),
      "new",
    ),
    undefined,
  );
  assert(
    projectReviewDiff(
      "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n",
      "new\n",
    ),
  );
  assert(
    projectReviewDiff(
      "@@ -1 +1 @@\n same\n\\ No newline at end of file\n",
      "same",
    ),
  );
  assertEquals(
    projectReviewDiff(`${noNewline}\\ No newline at end of file\n`, "new"),
    undefined,
  );
});
Deno.test("partial EOF, multiple files and unsupported binary/conflict formats are refused", () => {
  for (
    const invalid of [
      "diff --cc a.ts\n@@@ -1,1 -1,1 +1,1 @@@\n+new\n",
      "Binary files a/a and b/a differ\n",
      "\\ No newline at end of file\n",
      `diff --git a/other b/other\n${patch}`,
      patch.slice(0, -6),
    ]
  ) assertEquals(projectReviewDiff(invalid, source), undefined);
});
Deno.test("LF normalization is explicit and shared; BOM and NUL are not removed", () => {
  const raw = "@@ -0,0 +1 @@\r\n+\ufeffa\0\r\n";
  assertEquals(projectReviewDiff(raw, "\ufeffa\0\r\n"), undefined);
  assert(
    projectReviewDiff(
      reviewDisplayText(raw),
      reviewDisplayText("\ufeffa\0\r\n"),
    ),
  );
  assertEquals(projectReviewDiff(reviewDisplayText(raw), "a\0\n"), undefined);
});
Deno.test("projection bounds precede line allocation and malformed Unicode cannot be captured", () => {
  assertEquals(
    projectReviewDiff(patch, "x".repeat(4 * 1024 * 1024 + 1)),
    undefined,
  );
  assertEquals(projectReviewDiff("\n".repeat(100_000), source), undefined);
  assertEquals(projectReviewDiff(patch, "\n".repeat(100_000)), undefined);
  assertEquals(projectReviewDiff(patch, "\ud800"), undefined);
  assertEquals(
    projectReviewDiff("@@ -0,0 +1 @@\n+\ud800\n", "\ud800\n"),
    undefined,
  );
});
