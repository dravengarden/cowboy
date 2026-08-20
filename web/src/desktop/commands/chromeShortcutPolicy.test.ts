import { assert, assertEquals, assertThrows } from "jsr:@std/assert";
import {
  assertChromeShortcutAllowed,
  chromeShortcutConflict,
  INTENTIONAL_CHROME_VIM_OVERRIDES,
} from "./chromeShortcutPolicy.ts";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_COMMANDS,
  desktopWorkspacePrefix,
} from "./workspaceShortcuts.ts";

const providerSource = await Deno.readTextFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url),
);

Deno.test("Chrome tab, window, address, and numbered-tab chords are rejected", () => {
  for (
    const shortcut of [
      "Mod+N",
      "Mod+T",
      "Mod+W",
      "Mod+L",
      "Mod+E",
      "Mod+,",
      "Mod+[",
      "Mod+]",
      "Mod+0",
      "Mod+1",
      "Mod+5",
      "Mod+9",
      "Mod+Tab",
      "Alt+ArrowLeft",
      "Alt+ArrowRight",
      "Alt+Mod+J",
      "Shift+Escape",
      "F12",
    ]
  ) {
    assertThrows(() => assertChromeShortcutAllowed("test.command", shortcut, false));
  }
});

Deno.test("workspace prefix follows Chrome's platform-specific K behavior", () => {
  assertEquals(desktopWorkspacePrefix(true), "Mod+K");
  assertEquals(desktopWorkspacePrefix(false), "Alt+K");
  assertEquals(chromeShortcutConflict("workspace.prefix", "Mod+K", true), null);
  assertThrows(() => assertChromeShortcutAllowed("workspace.prefix", "Mod+K", false));
  assertEquals(chromeShortcutConflict("workspace.prefix", "Alt+K", false), null);
});

Deno.test("native save is the only registered semantic Chrome override", () => {
  assertEquals(chromeShortcutConflict("composer.saveDraft", "Mod+S", true), null);
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+S", true));
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+F", true));
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+J", true));
});

Deno.test("direct product chords and Alt session slots remain browser-safe", () => {
  for (const shortcut of ["Mod+Shift+P", "Mod+/", "Mod+.", "Alt+1", "Alt+0"]) {
    assertEquals(chromeShortcutConflict("test.command", shortcut, true), null);
  }
});

Deno.test("workspace navigation has no global bare-letter shortcut", () => {
  assertEquals(Object.keys(DESKTOP_WORKSPACE_COMMANDS).sort(), [
    ",", "c", "d", "l", "n", "p", "q", "r", "s", "t", "w",
  ]);
  for (const shortcut of Object.values(DESKTOP_SHORTCUTS)) {
    assert(!/^[a-z]$/i.test(shortcut));
  }
});

Deno.test("every registered Desktop command passes browser and product policy", () => {
  assert(providerSource.includes("assertChromeShortcutAllowed(command.id, command.shortcut, isMac)"));
  assert(providerSource.includes("assertShortcutRegistrationAllowed(command"));
  assert(providerSource.includes("matchesDesktopWorkspacePrefix(event)"));
});

Deno.test("intentional Chrome overrides are limited to reader Vim motions", () => {
  assertEquals(INTENTIONAL_CHROME_VIM_OVERRIDES, [
    "Ctrl+D",
    "Ctrl+U",
    "Ctrl+F",
    "Ctrl+B",
  ]);
});
