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
