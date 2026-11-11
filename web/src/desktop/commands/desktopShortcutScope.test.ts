import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { desktopOverlayOwnsShortcuts } from "./desktopShortcutScope.ts";

function root(match: boolean): { querySelector: () => unknown } {
  return { querySelector: () => match ? {} : null };
}

test("the focused overlay layer suspends workspace shortcuts", () => {
  assertEquals(desktopOverlayOwnsShortcuts(root(true)), true);
  assertEquals(desktopOverlayOwnsShortcuts(root(false)), false);
});

test("the remote Machine browser viewer owns every key while focused", async () => {
  const { remoteViewerOwnsKey } = await import("./desktopShortcutScope");
  const inside = { closest: (selector: string) => (selector === "[data-machine-browser-viewer]" ? {} : null) };
  const outside = { closest: () => null };
  assertEquals(remoteViewerOwnsKey({ target: inside as unknown as EventTarget }), true);
  assertEquals(remoteViewerOwnsKey({ target: outside as unknown as EventTarget }), false);
  assertEquals(remoteViewerOwnsKey({ target: null }), false);
  // The global router stands aside first: before input Vim, the workspace
  // prefix, Space and the modal leader can claim the key.
  const { readFile } = await import("node:fs/promises");
  const router = await readFile(new URL("./DesktopCommandProvider.tsx", import.meta.url), "utf8");
  const start = router.indexOf("const onKeyDown = (event: KeyboardEvent): void => {");
  const bypass = router.indexOf("if (remoteViewerOwnsKey(event)) return;", start);
  assertEquals(start >= 0 && bypass > start, true);
  for (const later of [
    "handleInputVimKey(event)",
    "desktopMenuNavigationKey(event)",
    "matchesDesktopWorkspacePrefix(event)",
    "isDesktopLeaderSpace(event)",
    'armWorkspaceCommand("modal")',
  ]) {
    assertEquals(router.indexOf(later, start) > bypass, true, later);
  }
});
