import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import {
  KEYSYM,
  keyActionForKey,
  keyActionsForInput,
  keyActionsForText,
  keysymForCharacter,
} from "./touchKeyboard";

test("characters map to Latin-1 or Unicode keysyms and control characters to none", () => {
  assertEquals(keysymForCharacter("a"), 0x61);
  assertEquals(keysymForCharacter(" "), 0x20);
  assertEquals(keysymForCharacter("é"), 0xe9);
  assertEquals(keysymForCharacter("界"), 0x01000000 | 0x754c);
  assertEquals(keysymForCharacter("😀"), 0x01000000 | 0x1f600);
  assertEquals(keysymForCharacter("\u0007"), undefined);
  assertEquals(keysymForCharacter("\u007f"), undefined);
  assertEquals(keysymForCharacter(""), undefined);
});

test("typed text becomes one press per code point with Return for line breaks", () => {
  assertEquals(keyActionsForText("ab\n界"), [
    { keysym: 0x61, code: null },
    { keysym: 0x62, code: null },
    { keysym: KEYSYM.Return, code: "Enter" },
    { keysym: 0x01000000 | 0x754c, code: null },
  ]);
  assertEquals(keyActionsForText("\r\n"), [
    { keysym: KEYSYM.Return, code: "Enter" },
    { keysym: KEYSYM.Return, code: "Enter" },
  ]);
});

test("keydown handles navigation keys but leaves Backspace to beforeinput", () => {
  assertEquals(keyActionForKey("Enter"), { keysym: KEYSYM.Return, code: "Enter" });
  assertEquals(keyActionForKey("ArrowLeft"), { keysym: KEYSYM.Left, code: "ArrowLeft" });
  assertEquals(keyActionForKey("Tab"), { keysym: KEYSYM.Tab, code: "Tab" });
  assertEquals(keyActionForKey("Backspace"), undefined);
  assertEquals(keyActionForKey("a"), undefined);
  assertEquals(keyActionForKey("constructor"), undefined);
});

test("input types translate to keys and compositions wait for their end", () => {
  assertEquals(keyActionsForInput("insertText", "hi"), [
    { keysym: 0x68, code: null },
    { keysym: 0x69, code: null },
  ]);
  assertEquals(keyActionsForInput("deleteContentBackward", null), [
    { keysym: KEYSYM.BackSpace, code: "Backspace" },
  ]);
  assertEquals(keyActionsForInput("deleteContentForward", null), [
    { keysym: KEYSYM.Delete, code: "Delete" },
  ]);
  assertEquals(keyActionsForInput("insertLineBreak", null), [
    { keysym: KEYSYM.Return, code: "Enter" },
  ]);
  assertEquals(keyActionsForInput("insertCompositionText", "中"), undefined);
  assertEquals(keyActionsForInput("formatBold", null), []);
});
