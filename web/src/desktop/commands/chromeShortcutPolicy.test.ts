import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals, assertThrows } from "@std/assert";
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

const providerSource = await readFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url), "utf8",
);

test("Chrome tab, window, address, and numbered-tab chords are rejected", () => {
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
    assertThrows(() =>
      assertChromeShortcutAllowed("test.command", shortcut, false)
    );
  }
});

test("workspace prefix follows Chrome's platform-specific K behavior", () => {
  assertEquals(desktopWorkspacePrefix(true), "Mod+K");
  assertEquals(desktopWorkspacePrefix(false), "Alt+K");
  assertEquals(chromeShortcutConflict("workspace.prefix", "Mod+K", true), null);
  assertThrows(() =>
    assertChromeShortcutAllowed("workspace.prefix", "Mod+K", false)
  );
  assertEquals(
    chromeShortcutConflict("workspace.prefix", "Alt+K", false),
    null,
  );
});

test("native save is the only registered semantic Chrome override", () => {
  assertEquals(
    chromeShortcutConflict("composer.saveDraft", "Mod+S", true),
    null,
  );
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+S", true));
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+F", true));
  assertThrows(() => assertChromeShortcutAllowed("unrelated", "Mod+J", true));
});

test("direct product chords remain browser-safe", () => {
  for (const shortcut of ["Mod+Shift+P", "Mod+/", "Mod+."]) {
    assertEquals(chromeShortcutConflict("test.command", shortcut, true), null);
  }
});

test("workspace navigation has no global bare-letter shortcut", () => {
  assertEquals(Object.keys(DESKTOP_WORKSPACE_COMMANDS).sort(), [
    " ",
    ",",
    ".",
    "/",
    "a",
    "c",
    "d",
    "f",
    "g",
    "h",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "tab",
    "u",
    "w",
    "z",
  ]);
  // Windows/Linux held-prefix pane folds (Alt + [ ] \) are not Chrome chords.
  // macOS Cmd+[ / Cmd+] are Chrome Back/Forward, which are not reserved
  // accelerators; the claimed continuation cancels them (FOCUS.md).
  for (const key of ["[", "]", "\\"]) {
    assertEquals(
      chromeShortcutConflict("test.command", `Alt+${key}`, false),
      null,
    );
  }
  for (const shortcut of Object.values(DESKTOP_SHORTCUTS)) {
    assert(!/^[a-z]$/i.test(shortcut));
  }
});

test("every registered Desktop command passes browser and product policy", () => {
  assert(
    providerSource.includes(
      "assertChromeShortcutAllowed(command.id, command.shortcut, isMac)",
    ),
  );
  assert(providerSource.includes("assertShortcutRegistrationAllowed(command"));
  assert(providerSource.includes("matchesDesktopWorkspacePrefix(event)"));
});

test("intentional Chrome overrides are limited to reader Vim motions", () => {
  assertEquals(INTENTIONAL_CHROME_VIM_OVERRIDES, [
    "Ctrl+D",
    "Ctrl+U",
    "Ctrl+F",
    "Ctrl+B",
    "Ctrl+H",
    "Ctrl+J",
    "Ctrl+K",
    "Ctrl+L",
  ]);
});

test("transactional edits and independent documents own native Save", () => {
  for (
    const id of [
      "pendingEdit.queued.done",
      "pendingEdit.draft.done",
      "document.save",
    ]
  ) {
    assertEquals(chromeShortcutConflict(id, "Mod+S", true), null);
  }
});
