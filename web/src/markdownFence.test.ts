import { assertEquals } from "jsr:@std/assert";
import { fencedCodeIsOpen } from "./markdownFence.ts";

Deno.test("a fence is open until a matching closing fence arrives", () => {
  assertEquals(fencedCodeIsOpen("```"), true);
  assertEquals(fencedCodeIsOpen("```text\nhalf a line"), true);
  assertEquals(fencedCodeIsOpen("```text\nline\n``"), true);
  assertEquals(fencedCodeIsOpen("```text\nline\n```"), false);
  assertEquals(fencedCodeIsOpen("```text\nline\n```  "), false);
});

Deno.test("only a same-character fence at least as long closes the block", () => {
  assertEquals(fencedCodeIsOpen("````md\n```\ninner\n```"), true);
  assertEquals(fencedCodeIsOpen("````md\n```\ninner\n```\n````"), false);
  assertEquals(fencedCodeIsOpen("~~~\ncode\n```"), true);
  assertEquals(fencedCodeIsOpen("~~~\ncode\n~~~"), false);
});

Deno.test("an indented code block is never open", () => {
  assertEquals(fencedCodeIsOpen("    indented code"), false);
});
