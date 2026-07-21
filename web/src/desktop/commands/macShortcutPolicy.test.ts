import { assertEquals, assertThrows } from "jsr:@std/assert";
import {
  assertMacShortcutAllowed,
  macShortcutConflict,
} from "./macShortcutPolicy";

Deno.test("macOS destructive and system shortcuts are rejected", () => {
  for (
    const shortcut of [
      "Mod+Q",
      "Mod+W",
      "Mod+H",
      "Mod+M",
      "Mod+Tab",
      "Mod+Space",
      "Ctrl+Mod+Q",
      "Ctrl+Mod+F",
      "Alt+Mod+Escape",
      "Shift+Mod+3",
      "Shift+Mod+4",
      "Shift+Mod+5",
      "Ctrl+Space",
      "Ctrl+Alt+Space",
      "Alt+E",
      "Alt+I",
      "Alt+N",
      "Alt+U",
    ]
  ) {
    assertThrows(() => assertMacShortcutAllowed("test.command", shortcut));
  }
});

Deno.test("common Command shortcuts require matching native semantics", () => {
  assertEquals(macShortcutConflict("session.new", "Mod+N"), null);
  assertEquals(macShortcutConflict("settings.open", "Mod+,"), null);
  assertEquals(macShortcutConflict("composer.saveDraft", "Mod+S"), null);
  assertEquals(macShortcutConflict("workspace.focusSessions", "Mod+B"), null);
  assertEquals(macShortcutConflict("workspace.focusTopbar", "Mod+T"), null);
  assertThrows(() => assertMacShortcutAllowed("unrelated", "Mod+S"));
});

Deno.test("Cowboy workspace chords avoid Command collisions", () => {
  for (const [commandId, shortcut] of [
    ["workspace.focusSessions", "Mod+B"],
    ["workspace.focusPrompt", "Mod+E"],
    ["workspace.focusConversation", "Mod+L"],
    ["workspace.focusTopbar", "Mod+T"],
    ["commandPalette.open", "Mod+K"],
    ["shortcuts.open", "Mod+/"],
  ] as const) {
    assertEquals(macShortcutConflict(commandId, shortcut), null);
  }
  assertThrows(() => assertMacShortcutAllowed("unrelated", "Mod+B"));
  assertThrows(() => assertMacShortcutAllowed("prompt.focusQueue", "Q"));
  assertEquals(macShortcutConflict("prompt.focusQueue", "Mod+Y"), null);
  assertEquals(macShortcutConflict("prompt.focusDrafts", "Mod+D"), null);
});
