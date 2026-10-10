import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  transcriptNeedsHydration,
  transcriptRetryDelay,
} from "./transcriptHydrationPolicy.ts";

test("a painted replica still needs server recovery after bootstrap failure", () => {
  assertEquals(transcriptNeedsHydration(true, "replica"), true);
  assertEquals(transcriptNeedsHydration(false, undefined), true);
  assertEquals(transcriptNeedsHydration(true, "live"), false);
  assertEquals(transcriptNeedsHydration(true, undefined), false);
});

test("transcript recovery backs off without permanently abandoning cached history", () => {
  assertEquals([0, 1, 2, 3, 99].map(transcriptRetryDelay), [
    750,
    2000,
    10000,
    30000,
    30000,
  ]);
});
