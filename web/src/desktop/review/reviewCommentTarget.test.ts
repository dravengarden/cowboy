import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { isMac } from "../../platform.ts";
import { resolveDiffCommentLocation } from "./reviewCommentTarget.ts";
import { reviewNoteKeyAction } from "./ReviewInlineCommentBar.tsx";
import { streamEntryCacheKey } from "./ReviewAllChangesStream.tsx";
import type { GitReviewEntry } from "../../review/gitReviewModel.ts";

const SAMPLE_DIFF = [
  "diff --git a/src/foo.ts b/src/foo.ts",
  "--- a/src/foo.ts",
  "+++ b/src/foo.ts",
  "@@ -200,4 +210,5 @@ export function demo() {",
  " const keep = 1;",
  "-const removed = 2;",
  "+const addedA = 3;",
  "+const addedB = 4;",
  " return keep;",
].join("\n");

test("resolveDiffCommentLocation maps unified patch rows to real file lines and sides", () => {
  // Patch rows 7..8 correspond to new-side file lines 211..212 (not patch rows 7..8!).
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 7, to: 8 }), {
    startLine: 211,
    endLine: 212,
    side: "new",
  });
  // Deleted-only row 6 maps to old-side line 201.
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 6, to: 6 }), {
    startLine: 201,
    endLine: 201,
    side: "old",
  });
  // Hunk header row 4 has no file line -> falls back cleanly to patch row.
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 4, to: 4 }), {
    startLine: 4,
    endLine: 4,
    side: "patch",
  });
});

test("reviewNoteKeyAction honors IME active/post-composition ownership and platform Mod+Enter", () => {
  const base = {
    key: "Enter",
    code: "Enter",
    keyCode: 13,
    isComposing: false,
    metaKey: isMac,
    ctrlKey: !isMac,
    altKey: false,
    shiftKey: false,
    target: null,
  };
  assertEquals(reviewNoteKeyAction(base, { composing: false }), "save");
  // Post-composition hold window (isComposing already false, composing=true) MUST NOT stage!
  assertEquals(reviewNoteKeyAction(base, { composing: true }), "ignore");
  assertEquals(reviewNoteKeyAction({ ...base, isComposing: true }, { composing: false }), "ignore");
  assertEquals(
    reviewNoteKeyAction({ ...base, key: "Escape", code: "Escape", metaKey: false, ctrlKey: false }, { composing: false }),
    "cancel",
  );
  assertEquals(
    reviewNoteKeyAction({ ...base, key: "Escape", code: "Escape", metaKey: false, ctrlKey: false }, { composing: true }),
    "ignore",
  );
});

test("streamEntryCacheKey isolates different sessions, fingerprints, settings, and revisions", () => {
  const entry: GitReviewEntry = {
    change: { path: "src/app.ts", status: "modified", staged: false, unstaged: true, fingerprint: "fp-1" },
    scope: "unstaged",
    comparison: "origin/main",
  };
  const a = streamEntryCacheKey("sess-a", entry, 3, false, 1);
  const b = streamEntryCacheKey("sess-b", entry, 3, false, 1);
  const rev2 = streamEntryCacheKey("sess-a", entry, 3, false, 2);
  const fp2 = streamEntryCacheKey("sess-a", { ...entry, change: { ...entry.change, fingerprint: "fp-2" } }, 3, false, 1);
  assertEquals(a === b, false);
  assertEquals(a === rev2, false);
  assertEquals(a === fp2, false);
});
