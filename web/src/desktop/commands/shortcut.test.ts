import { assert, assertEquals, assertFalse } from "jsr:@std/assert";
import { matchesShortcut, parseShortcut } from "./shortcut";

Deno.test("shortcut parser normalizes a desktop chord", () => {
  assertEquals(parseShortcut("Mod+Shift+P"), {
    key: "p",
    mod: true,
    ctrl: false,
    shift: true,
    alt: false,
  });
});

Deno.test("Ctrl remains Control on every desktop platform", () => {
  const stroke = parseShortcut("Ctrl+4");
  assertEquals(stroke, {
    key: "4",
    mod: false,
    ctrl: true,
    shift: false,
    alt: false,
  });
  assert(matchesShortcut(stroke, {
    key: "4",
    code: "Digit4",
    metaKey: false,
    ctrlKey: true,
    shiftKey: false,
    altKey: false,
  }, true));
  assertFalse(matchesShortcut(stroke, {
    key: "4",
    code: "Digit4",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
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
  assert(matchesShortcut(parseShortcut("Mod+2"), {
    key: "™",
    code: "Digit2",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
});

Deno.test("Alt shortcuts use physical keys under macOS Option input", () => {
  assert(matchesShortcut(parseShortcut("Alt+2"), {
    key: "™",
    code: "Digit2",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: true,
  }, true));
  assert(matchesShortcut(parseShortcut("Alt+A"), {
    key: "å",
    code: "KeyA",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: true,
  }, true));
  for (const [shortcut, key, code] of [
    ["Alt+P", "π", "KeyP"],
    ["Alt+D", "∂", "KeyD"],
    ["Alt+E", "Dead", "KeyE"],
    ["Alt+C", "ç", "KeyC"],
    ["Alt+S", "ß", "KeyS"],
    ["Alt+T", "†", "KeyT"],
  ] as const) {
    assert(matchesShortcut(parseShortcut(shortcut), {
      key,
      code,
      metaKey: false,
      ctrlKey: false,
      shiftKey: false,
      altKey: true,
    }, true));
  }
  assert(matchesShortcut(parseShortcut("Alt+/"), {
    key: "÷",
    code: "Slash",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: true,
  }, true));
});

Deno.test("Mod backslash enters Resize mode under an IME", () => {
  assert(matchesShortcut(parseShortcut("Mod+\\"), {
    key: "Process",
    code: "Backslash",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
});

Deno.test("Mod bracket resize shortcuts use physical keys under an IME", () => {
  assert(matchesShortcut(parseShortcut("Mod+["), {
    key: "Process",
    code: "BracketLeft",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
  assert(matchesShortcut(parseShortcut("Mod+]"), {
    key: "Process",
    code: "BracketRight",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
});

Deno.test("bare product letters ignore Shift; modified chords do not", () => {
  const follow = parseShortcut("F");
  const shiftedF = {
    key: "F",
    code: "KeyF",
    metaKey: false,
    ctrlKey: false,
    shiftKey: true,
    altKey: false,
  };
  assert(matchesShortcut(follow, shiftedF, true, true));
  assert(matchesShortcut(follow, { ...shiftedF, key: "f", shiftKey: false }, true, true));
  assert(matchesShortcut(parseShortcut("Mod+I"), {
    key: "i",
    code: "KeyI",
    metaKey: true,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  }, true));
  assertFalse(matchesShortcut(parseShortcut("Mod+I"), {
    key: "I",
    code: "KeyI",
    metaKey: true,
    ctrlKey: false,
    shiftKey: true,
    altKey: false,
  }, true));
  assert(matchesShortcut(parseShortcut("Mod+Shift+P"), {
    key: "P",
    code: "KeyP",
    metaKey: true,
    ctrlKey: false,
    shiftKey: true,
    altKey: false,
  }, true));
});

Deno.test("bare contextual shortcuts use physical keys only when explicitly safe", () => {
  const shortcut = parseShortcut("P");
  const imeKey = {
    key: "Process",
    code: "KeyP",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  };
  assertFalse(matchesShortcut(shortcut, imeKey, true));
  assert(matchesShortcut(shortcut, imeKey, true, true));
});
