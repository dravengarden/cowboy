import { assertEquals } from "jsr:@std/assert";
import {
  findChar,
  normalClamp,
  operatorRange,
  removeRange,
  lineRange,
  verticalMove,
  wordBackward,
  wordEnd,
  wordForward,
} from "./inputVimEdit.ts";

const text = "foo.bar  baz";

Deno.test("word motions split words from punctuation; W/B/E by blanks", () => {
  assertEquals(wordForward(text, 0), 3);
  assertEquals(wordForward(text, 3), 4);
  assertEquals(wordForward(text, 0, true), 9);
  assertEquals(wordBackward(text, 9), 4);
  assertEquals(wordBackward(text, 9, true), 0);
  assertEquals(wordEnd(text, 0), 2);
  assertEquals(wordEnd(text, 0, true), 6);
});

Deno.test("f/t find within the line", () => {
  assertEquals(findChar(text, 0, "f", "b"), 4);
  assertEquals(findChar(text, 0, "t", "b"), 3);
  assertEquals(findChar(text, 11, "F", "b"), 9);
  assertEquals(findChar(text, 11, "T", "b"), 10);
  assertEquals(findChar(text, 0, "f", "q"), null);
});

Deno.test("operators follow Vim's inclusive rules", () => {
  assertEquals(operatorRange(text, 0, "w", 3, "d"), { from: 0, to: 3 });
  assertEquals(operatorRange(text, 0, "e", 2, "d"), { from: 0, to: 3 });
  // cw changes to the end of the word, not into the blanks after it.
  assertEquals(operatorRange("ab  cd", 0, "w", 4, "c"), { from: 0, to: 2 });
  assertEquals(operatorRange(text, 4, "$", 11, "d"), { from: 4, to: 12 });
});

Deno.test("Normal cursor and textarea lines", () => {
  assertEquals(normalClamp("abc", 3), 2);
  assertEquals(normalClamp("", 0), 0);
  assertEquals(verticalMove("ab\ncdef", 1, 1), 4);
  assertEquals(verticalMove("abcd\nx", 3, 1), 5);
  assertEquals(verticalMove("ab\ncd", 4, -1), 1);
  assertEquals(verticalMove("ab", 0, -1), null);
  assertEquals(removeRange("a\nb\nc", lineRange("a\nb\nc", 2)), "a\nc");
  assertEquals(removeRange("a\nb", lineRange("a\nb", 2)), "a");
});
