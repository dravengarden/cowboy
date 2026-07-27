import { assertEquals } from "jsr:@std/assert";
import { normalizeReviewMode } from "./reviewMode.ts";

Deno.test("review mode accepts only the two local surface states", () => {
  assertEquals(normalizeReviewMode("files"), "files");
  assertEquals(normalizeReviewMode("code"), "files");
  assertEquals(normalizeReviewMode("git"), "git");
  assertEquals(normalizeReviewMode("unknown"), "git");
  assertEquals(normalizeReviewMode(undefined), "git");
});
