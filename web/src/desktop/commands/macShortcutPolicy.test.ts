import { assertEquals, assertThrows } from "jsr:@std/assert";
import {
  assertMacShortcutAllowed,
  macShortcutConflict,
} from "./macShortcutPolicy.ts";

Deno.test("macOS destructive and system shortcuts are rejected", () => {
  for (
    const shortcut of [
      "Mod+Q", "Mod+W", "Mod+H", "Mod+M", "Mod+Tab", "Mod+Space",
      "Ctrl+Mod+Q", "Ctrl+Mod+F", "Alt+Mod+Escape", "Shift+Mod+3",
      "Shift+Mod+4", "Shift+Mod+5", "Ctrl+Space", "Ctrl+Alt+Space",
      "Alt+E", "Alt+I", "Alt+N", "Alt+U",
    ]
  ) {
    assertThrows(() => assertMacShortcutAllowed("test.command", shortcut));
  }
});

Deno.test("common Command shortcuts require matching product semantics", () => {
  assertEquals(macShortcutConflict("composer.saveDraft", "Mod+S"), null);
  assertEquals(macShortcutConflict("topbar.stop", "Mod+."), null);
  assertEquals(macShortcutConflict("workspace.prefix", "Mod+K"), null);
  assertThrows(() => assertMacShortcutAllowed("composer.more", "Mod+."));
  assertThrows(() => assertMacShortcutAllowed("unrelated", "Mod+S"));
  assertThrows(() => assertMacShortcutAllowed("unrelated", "Mod+K"));
  assertThrows(() => assertMacShortcutAllowed("settings.open", "Mod+,"));
});

Deno.test("macOS direct and fallback workspace chords are safe", () => {
  for (const [commandId, shortcut] of [
    ["commandPalette.open", "Mod+Shift+P"],
    ["shortcuts.open", "Mod+/"],
    ["session.slot1", "Alt+1"],
    ["session.slot0", "Alt+0"],
    ["workspace.prefix", "Alt+K"],
  ] as const) {
    assertEquals(macShortcutConflict(commandId, shortcut), null);
  }
  assertThrows(() => assertMacShortcutAllowed("prompt.focusQueue", "Q"));
});
