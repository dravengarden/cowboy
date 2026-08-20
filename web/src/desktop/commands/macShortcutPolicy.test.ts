import { assertEquals, assertThrows } from "jsr:@std/assert";
import {
  assertMacShortcutAllowed,
  macShortcutConflict,
} from "./macShortcutPolicy";
import {
  DESKTOP_FOCUS_PLAN_SHORTCUT,
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
} from "./workspaceShortcuts";

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
  assertEquals(macShortcutConflict("composer.saveDraft", "Mod+S"), null);
  assertEquals(macShortcutConflict("composer.more", "Mod+."), null);
  assertThrows(() => assertMacShortcutAllowed("unrelated", "Mod+S"));
  assertThrows(() => assertMacShortcutAllowed("settings.open", "Mod+,"));
});

Deno.test("Cowboy workspace keys avoid Command collisions", () => {
  for (const [commandId, shortcut] of [
    ["workspace.focusSessions", "S"],
    ["workspace.focusPrompt", DESKTOP_FOCUS_PROMPT_SHORTCUT],
    ["prompt.focusPlan", DESKTOP_FOCUS_PLAN_SHORTCUT],
    ["workspace.focusConversation", "C"],
    ["workspace.focusTopbar", "T"],
    ["commandPalette.open", ":"],
    ["shortcuts.open", "?"],
    ["workspace.enterResize", "\\"],
    ["session.slot1", "Alt+1"],
    ["session.slot0", "Alt+0"],
  ] as const) {
    assertEquals(macShortcutConflict(commandId, shortcut), null);
  }
  assertThrows(() => assertMacShortcutAllowed("unrelated", "Mod+E"));
  assertThrows(() => assertMacShortcutAllowed("prompt.focusQueue", "Q"));
  assertEquals(macShortcutConflict("prompt.focusQueue", "Y"), null);
  assertEquals(macShortcutConflict("prompt.focusDrafts", "D"), null);
});
