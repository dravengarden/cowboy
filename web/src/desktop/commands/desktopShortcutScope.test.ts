import { assertEquals } from "jsr:@std/assert";
import { desktopOverlayOwnsShortcuts } from "./desktopShortcutScope.ts";

function root(match: boolean): { querySelector: () => unknown } {
  return { querySelector: () => match ? {} : null };
}

Deno.test("an exclusive overlay suspends workspace Vim shortcuts", () => {
  assertEquals(desktopOverlayOwnsShortcuts(root(true)), true);
  assertEquals(desktopOverlayOwnsShortcuts(root(false)), false);
});
