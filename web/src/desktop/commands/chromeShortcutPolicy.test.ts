import { assertEquals, assertThrows } from "jsr:@std/assert";
import {
  assertChromeShortcutAllowed,
  chromeShortcutConflict,
  INTENTIONAL_CHROME_VIM_OVERRIDES,
} from "./chromeShortcutPolicy.ts";
import { DESKTOP_SHORTCUTS } from "./workspaceShortcuts.ts";

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
      "Mod+K",
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
    assertThrows(() => assertChromeShortcutAllowed("test.command", shortcut));
  }
});

Deno.test("native save is the only registered semantic Chrome override", () => {
  assertEquals(chromeShortcutConflict("composer.saveDraft", "Mod+S"), null);
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+S"));
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+F"));
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+J"));
});

Deno.test("bare workspace keys and Alt session slots remain browser-safe", () => {
  for (const shortcut of ["S", "E", "P", "C", "T", "N", ":", "?", "Alt+1", "Alt+0"]) {
    assertEquals(chromeShortcutConflict("test.command", shortcut), null);
  }
});

Deno.test("the canonical workspace map stays direct and Chrome-safe", () => {
  assertEquals(DESKTOP_SHORTCUTS, {
    shortcuts: "?",
    commands: ":",
    newSession: "N",
    settings: ",",
    focusTopbar: "T",
    focusSessions: "S",
    focusPrompt: "E",
    focusConversation: "C",
    focusPlan: "P",
    focusQueue: "Y",
    focusDrafts: "D",
    cycleRegion: "W",
    resize: "\\",
    sessionSlots: "Alt+1…0",
  });
});

Deno.test("every registered Desktop command passes the Chrome policy", () => {
  assertEquals(
    providerSource.includes("assertChromeShortcutAllowed(command.id, command.shortcut)"),
    true,
  );
  assertEquals(providerSource.includes("desktopBrowserChromeShortcut"), false);
});

Deno.test("intentional Chrome overrides are limited to reader Vim motions", () => {
  assertEquals(INTENTIONAL_CHROME_VIM_OVERRIDES, [
    "Ctrl+D",
    "Ctrl+U",
    "Ctrl+F",
    "Ctrl+B",
  ]);
});
