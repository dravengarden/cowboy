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
  selectActiveStreamRows,
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
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 9, to: 10 }), { startLine: 211, endLine: 212, side: "new" });
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 6, to: 7 }), { startLine: 201, endLine: 202, side: "old" });
  assertEquals(resolveDiffCommentLocation(SAMPLE_DIFF, { from: 4, to: 4 }), { startLine: 4, endLine: 4, side: "patch" });
});

test("resolveSelectedUnifiedRows resolves non-monotonic split rows in a single pass without skipped lines", () => {
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
    ownerSessionId: "a", reviewSessionId: "r", kind: "diff" as const, path: "src/app.ts",
    startLine: 10, endLine: 10, side: "new" as const, scope: "staged" as const, comparison: "origin/main", revision: "rev1",
  };
  assertEquals(commentTargetIdentity(base) === commentTargetIdentity({ ...base, scope: "unstaged" }), false);
  assertEquals(commentTargetIdentity(base) === commentTargetIdentity({ ...base, comparison: "HEAD" }), false);
  assertEquals(commentTargetIdentity(base) === commentTargetIdentity({ ...base, revision: "rev2" }), false);
});

test("reviewNoteKeyAction handles IME Escape and platform Mod+Enter", () => {
  const base = {
    key: "Enter", code: "Enter", keyCode: 13, isComposing: false,
    metaKey: isMac, ctrlKey: !isMac, altKey: false, shiftKey: false, target: null,
  };
  assertEquals(reviewNoteKeyAction(base, { composing: false }), "save");
  assertEquals(reviewNoteKeyAction(base, { composing: true }), "ignore");
  assertEquals(reviewNoteKeyAction({ ...base, key: "Escape", code: "Escape", metaKey: false, ctrlKey: false }, { composing: false }), "cancel");
  assertEquals(reviewNoteKeyAction({ ...base, key: "Escape", code: "Escape", metaKey: false, ctrlKey: false }, { composing: true }), "ime-escape");
});

test("fingerprint-bound stale check, multi-page truncation, promoted Reload diff, and bounded cache", () => {
  assertEquals(isDiffSnapshotTruncated(undefined, { truncated: false, limited: false, nextCursor: undefined }), false);
  assertEquals(isDiffSnapshotTruncated("cursor-5", { truncated: true, limited: false, nextCursor: "cursor-5" }), true);
  assertEquals(isDiffSnapshotTruncated(undefined, { truncated: true, limited: true, nextCursor: undefined }), true);

  const entry: GitReviewEntry = {
    change: { path: "src/app.ts", status: "modified", staged: false, unstaged: true, fingerprint: "fp-2", additions: 5, deletions: 1 },
    scope: "combined",
    comparison: "origin/main",
  };
  // First open with missing or mismatched changeFingerprint MUST be flagged stale!
  assertEquals(isStreamDiffStale(entry, { revision: "rev-old", added: 5, removed: 1 }, undefined), true);
  assertEquals(isStreamDiffStale(entry, { revision: "rev-old", changeFingerprint: "fp-1", added: 5, removed: 1 }, undefined), true);
  // Matching changeFingerprint + matching line counts -> verified fresh!
  assertEquals(isStreamDiffStale(entry, { revision: "rev-new", changeFingerprint: "fp-2", added: 5, removed: 1, truncated: false }, undefined), false);

  const a = streamEntryCacheKey("sess-a", entry, 3, false, 1);
  const b = streamEntryCacheKey("sess-b", entry, 3, false, 1);
  assertEquals(a === b, false);

  // Promoting an evicted old row via Reload diff puts it inside the 30-row active fetch set!
  const rows = Array.from({ length: 40 }, (_, i) => ({ key: `f-${String(i)}` }));
  const promoted = selectActiveStreamRows(rows, ["f-0"], 30);
  assertEquals(promoted.length, 30);
  assertEquals(promoted.some((r) => r.key === "f-0"), true);

  const big = new Map<string, StreamDiffData>();
  const keep = new Set<string>();
  for (let i = 0; i < 45; i++) {
    const k = `sess-a\0k${String(i)}`;
    big.set(k, { state: "ready", text: "x" });
    keep.add(k);
  }
  assertEquals(pruneStreamCacheBounded(big, "sess-a\0", keep, 30).size, 30);
});
