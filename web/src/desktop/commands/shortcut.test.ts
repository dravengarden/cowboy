import { assert, assertEquals, assertFalse } from "jsr:@std/assert";
import { matchesShortcut, parseShortcut } from "./shortcut";

Deno.test("shortcut parser normalizes a desktop chord", () => {
  assertEquals(parseShortcut("Mod+Shift+P"), {
    key: "p",
    mod: true,
    shift: true,
    alt: false,
  });
});

Deno.test("Mod maps to Command on macOS and Control elsewhere", () => {
  const stroke = parseShortcut("Mod+K");
  assert(matchesShortcut(stroke, {
    key: "k",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
  assert(matchesShortcut(stroke, {
    key: "K",
    metaKey: false,
    ctrlKey: true,
    shiftKey: false,
    altKey: false,
  }, false));
  assertFalse(matchesShortcut(stroke, {
    key: "k",
    metaKey: true,
    ctrlKey: true,
    shiftKey: false,
    altKey: false,
  }, true));
});

Deno.test("numbered session shortcuts preserve digit identity", () => {
  const first = parseShortcut("Mod+1");
  const tenth = parseShortcut("Mod+0");
  assert(matchesShortcut(first, {
    key: "1",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
  assert(matchesShortcut(tenth, {
    key: "0",
    metaKey: false,
    ctrlKey: true,
    shiftKey: false,
    altKey: false,
  }, false));
});
