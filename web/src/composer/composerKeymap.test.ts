import { test } from "bun:test";
import { assert, assertEquals } from "../../../tools/test-assert.ts";
import { composerDefaultKeymap } from "./composerKeymap.ts";

test("composerDefaultKeymap excludes Mod-Enter, Mod-i, and Shift-Mod-k", () => {
  for (const blocked of ["Mod-Enter", "Mod-i", "Shift-Mod-k"]) {
    assertEquals(
      composerDefaultKeymap.some((b) =>
        b.key === blocked || b.mac === blocked || b.linux === blocked
      ),
      false,
      `${blocked} must not remain in composerDefaultKeymap`,
    );
  }
  // Core navigation and editing bindings remain intact.
  assert(composerDefaultKeymap.some((b) => b.key === "Enter"));
  assert(composerDefaultKeymap.some((b) => b.key === "ArrowUp"));
});
