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
  assertThrows(() =>
    assertMacShortcutAllowed("workspace.focusSessions", "Mod+B")
  );
  assertThrows(() =>
    assertMacShortcutAllowed("workspace.focusConversation", "Mod+T")
  );
  assertThrows(() => assertMacShortcutAllowed("unrelated", "Mod+S"));
});

Deno.test("Cowboy workspace chords avoid Command collisions", () => {
  for (
    const shortcut of [
      "Shift+Alt+Mod+B",
      "Shift+Alt+Mod+P",
      "Shift+Alt+Mod+C",
      "Shift+Alt+Mod+T",
      "Shift+Alt+Mod+R",
      "Shift+Alt+Mod+U",
      "Shift+Alt+Mod+K",
      "Shift+Alt+Mod+S",
      "Mod+K",
      "Mod+/",
    ]
  ) {
    assertEquals(macShortcutConflict("workspace.command", shortcut), null);
  }
  assertThrows(() => assertMacShortcutAllowed("prompt.focusQueue", "Q"));
  assertEquals(macShortcutConflict("prompt.focusQueue", "O"), null);
});
