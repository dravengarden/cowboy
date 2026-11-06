import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { CallReadEpoch } from "./callReadEpoch";

test("active call completion and unrelated rerenders cannot trigger another read", () => {
  const reads = new CallReadEpoch();
  assertEquals(reads.claim("a", 100, "old-call"), true);
  assertEquals(reads.claim("a", 100, "other-call"), true);
  assertEquals(reads.claim("a", 100, "old-call"), false);
  // A failed request retries at the next ledger observation too.
  assertEquals(reads.claim("a", 101, "old-call"), true);
  assertEquals(reads.claim("b", 101, "old-call"), true);
});
