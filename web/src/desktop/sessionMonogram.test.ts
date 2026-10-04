import { assertEquals } from "jsr:@std/assert";
import { sessionMonogram } from "./sessionMonogram.ts";

Deno.test("two-word titles use both initials", () => {
  assertEquals(sessionMonogram("codex debug"), "CD");
  assertEquals(sessionMonogram("stormbird RG deploy"), "SR");
  assertEquals(sessionMonogram("CS-PRO-001 codex"), "CP");
});

Deno.test("single-word titles keep the first two glyphs", () => {
  assertEquals(sessionMonogram("Cardea"), "Ca");
  assertEquals(sessionMonogram("matrix"), "Ma");
  assertEquals(sessionMonogram("修复登录"), "修复");
});

Deno.test("punctuation and empty titles stay readable", () => {
  assertEquals(sessionMonogram("  (draft) plan"), "DP");
  assertEquals(sessionMonogram(""), "?");
  assertEquals(sessionMonogram("--"), "?");
});
