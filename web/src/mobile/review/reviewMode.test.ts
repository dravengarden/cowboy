import { assertEquals } from "jsr:@std/assert";
import { normalizeReviewMode } from "./reviewMode.ts";

Deno.test("review mode accepts only the two local surface states", () => {
  assertEquals(normalizeReviewMode("code"), "code");
  assertEquals(normalizeReviewMode("git"), "git");
  assertEquals(normalizeReviewMode("files"), "git");
  assertEquals(normalizeReviewMode(undefined), "git");
});
