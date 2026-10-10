import { test } from "bun:test";
import { strict as assert } from "node:assert";
import { desktopShortcutCaps } from "./DesktopKeycap";
import { DESKTOP_SHORTCUTS } from "./workspaceShortcuts";

test("hover hints draw a leader path as one keycap, not the prefix string", () => {
  assert.deepEqual(desktopShortcutCaps(DESKTOP_SHORTCUTS.resize), ["␣WR"]);
  assert.deepEqual(desktopShortcutCaps(DESKTOP_SHORTCUTS.alternateSession), [
    "␣⇥",
  ]);
});

test("hover hints compact direct chords and named keys", () => {
  assert.deepEqual(desktopShortcutCaps("Shift+J"), ["⇧J"]);
  assert.deepEqual(desktopShortcutCaps("Escape"), ["Esc"]);
  assert.deepEqual(desktopShortcutCaps("O"), ["O"]);
});
