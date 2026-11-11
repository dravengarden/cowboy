import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { pastedAddress } from "./pasteAddress";

test("a single web link is offered as an address, trimmed", () => {
  assertEquals(
    pastedAddress("  https://claude.ai/oauth/authorize?code=true&state=x\n"),
    "https://claude.ai/oauth/authorize?code=true&state=x",
  );
  assertEquals(pastedAddress("http://example.com/"), "http://example.com/");
});

test("anything other than one web link is only pasted", () => {
  for (const text of [
    "",
    "   ",
    "hello",
    "javascript:alert(1)",
    "file:///etc/passwd",
    "https://a.example/ https://b.example/",
    "see https://a.example/",
    "https://a.example/\nmore",
  ]) {
    assertEquals(pastedAddress(text), undefined);
  }
});
