import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { isMac } from "../../platform.ts";
import type { GitReviewEntry } from "../../review/gitReviewModel.ts";
import {
  commentTargetIdentity,
  resolveDiffCommentLocation,
  resolveSelectedUnifiedRows,
} from "./reviewCommentTarget.ts";
import { reviewNoteKeyAction } from "./ReviewInlineCommentBar.tsx";
import {
  isDiffSnapshotTruncated,
  isStreamDiffStale,
  pruneStreamCacheBounded,
  streamEntryCacheKey,
  type StreamDiffData,
} from "./reviewStreamFetch.ts";

const SAMPLE_DIFF = [
  "diff --git a/src/foo.ts b/src/foo.ts",
  "--- a/src/foo.ts",
  "+++ b/src/foo.ts",
  "@@ -200,5 +210,4 @@ export function demo() {",
  " const keep = 1;",
  "-const del1 = 1;",
  "-const del2 = 2;",
  "-const del3 = 3;",
  "+const add1 = 10;",
  "+const add2 = 20;",
  " return keep;",
].join("\n");

test("resolveDiffCommentLocation maps unified patch rows to real file lines and sides", () => {
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 9, to: 10 }), {
    startLine: 211,
    endLine: 212,
    side: "new",
  });
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 6, to: 7 }), {
    startLine: 201,
    endLine: 202,
    side: "old",
  });
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 4, to: 4 }), {
    startLine: 4,
    endLine: 4,
    side: "patch",
  });
});

test("resolveSelectedUnifiedRows resolves non-monotonic split rows without pulling in skipped additions", () => {
  // In a 3-delete + 2-add hunk, split rows 2..3 map to unified rows [10 (add2), 8 (del3)].
  // Row 9 (add1) was NOT selected and must NOT appear in the snippet or line anchor!
  const res = resolveSelectedUnifiedRows(SAMPLE_DIFF, [10, 8]);
  assertEquals(res.startLine, 212);
  assertEquals(res.endLine, 212);
  assertEquals(res.side, "new");
  assertEquals(res.snippet.includes("add1"), false);
  assertEquals(res.snippet.includes("del3"), true);
  assertEquals(res.snippet.includes("add2"), true);
});

test("commentTargetIdentity distinguishes scope, comparison, kind, and revision on identical lines", () => {
  const base = {
    ownerSessionId: "a",
    reviewSessionId: "r",
    kind: "diff" as const,
    path: "src/app.ts",
    startLine: 10,
    endLine: 10,
    side: "new" as const,
    scope: "staged" as const,
    comparison: "origin/main",
    revision: "rev1",
  };
  assertEquals(commentTargetIdentity(base) === commentTargetIdentity({ ...base, scope: "unstaged" }), false);
  assertEquals(commentTargetIdentity(base) === commentTargetIdentity({ ...base, comparison: "HEAD" }), false);
  assertEquals(commentTargetIdentity(base) === commentTargetIdentity({ ...base, revision: "rev2" }), false);
});

test("reviewNoteKeyAction distinguishes IME-owned Escape from plain Escape and honors platform Mod+Enter", () => {
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
  assertEquals(reviewNoteKeyAction(base, { composing: true }), "ignore");
  assertEquals(reviewNoteKeyAction({ ...base, isComposing: true }, { composing: false }), "ignore");
  assertEquals(
    reviewNoteKeyAction({ ...base, key: "Escape", code: "Escape", metaKey: false, ctrlKey: false }, { composing: false }),
    "cancel",
  );
  // IME-owned Escape returns "ime-escape" so Popover/ancestors stopPropagation without preventDefault!
  assertEquals(
    reviewNoteKeyAction({ ...base, key: "Escape", code: "Escape", metaKey: false, ctrlKey: false }, { composing: true }),
    "ime-escape",
  );
});

test("multi-page truncation, stale snapshot detection, and bounded stream cache enforce invariants", () => {
  // Multi-page diff whose last page cleared nextCursor and limited=false is NOT truncated.
  assertEquals(isDiffSnapshotTruncated(undefined, { truncated: false, limited: false, nextCursor: undefined }), false);
  // Stopped at page budget with remaining cursor IS truncated.
  assertEquals(isDiffSnapshotTruncated("cursor-5", { truncated: true, limited: false, nextCursor: "cursor-5" }), true);
  // Server-limited 16 MiB cap IS truncated even without nextCursor.
  assertEquals(isDiffSnapshotTruncated(undefined, { truncated: true, limited: true, nextCursor: undefined }), true);

  const entry: GitReviewEntry = {
    change: { path: "src/app.ts", status: "modified", staged: false, unstaged: true, fingerprint: "fp-2", additions: 5, deletions: 1 },
    scope: "combined",
    comparison: "origin/main",
  };
  // New fingerprint with old patch revision -> stale!
  assertEquals(isStreamDiffStale(entry, { revision: "rev-old", added: 5, removed: 1 }, { fingerprint: "fp-1", revision: "rev-old" }), true);
  // Combined untruncated patch whose additions disagree with entry -> stale!
  assertEquals(isStreamDiffStale(entry, { revision: "rev-new", added: 2, removed: 1, truncated: false }, undefined), true);
  assertEquals(isStreamDiffStale(entry, { revision: "rev-new", added: 5, removed: 1, truncated: false }, { fingerprint: "fp-1", revision: "rev-old" }), false);

  // Cache key isolation
  const a = streamEntryCacheKey("sess-a", entry, 3, false, 1);
  const b = streamEntryCacheKey("sess-b", entry, 3, false, 1);
  assertEquals(a === b, false);

  // Strict bounded eviction even when preferKeep exceeds maxEntries
  const big = new Map<string, StreamDiffData>();
  const keep = new Set<string>();
  for (let i = 0; i < 45; i++) {
    const k = `sess-a\0k${String(i)}`;
    big.set(k, { state: "ready", text: "x" });
    keep.add(k);
  }
  const pruned = pruneStreamCacheBounded(big, "sess-a\0", keep, 30);
  assertEquals(pruned.size, 30);
});
