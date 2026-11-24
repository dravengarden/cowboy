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

test("a leader Space's keyup is swallowed, never one the remote viewer received", async () => {
  const { swallowLeaderSpaceKeyUp, machineBrowserOwnsKeys } = await import("./desktopShortcutScope");
  const inside = { closest: (selector: string) => (selector === "[data-machine-browser-viewer]" ? {} : null) };
  const outside = { closest: () => null };
  const keyUp = (target: unknown) => {
    const event = {
      code: "Space",
      target,
      prevented: false,
      preventDefault() {
        this.prevented = true;
      },
      stopImmediatePropagation() {},
    };
    return event;
  };
  const armed = { current: true };
  const handler = swallowLeaderSpaceKeyUp(armed);
  const remote = keyUp(inside);
  handler(remote as unknown as KeyboardEvent);
  assertEquals(remote.prevented, false);
  assertEquals(armed.current, false);
  armed.current = true;
  const chrome = keyUp(outside);
  handler(chrome as unknown as KeyboardEvent);
  assertEquals(chrome.prevented, true);
  assertEquals(armed.current, false);
  // Surfaces below the Machine browser stand aside while it is open.
  assertEquals(machineBrowserOwnsKeys({ target: outside as unknown as EventTarget }, root(true)), true);
  assertEquals(machineBrowserOwnsKeys({ target: inside as unknown as EventTarget }, root(false)), true);
  assertEquals(machineBrowserOwnsKeys({ target: outside as unknown as EventTarget }, root(false)), false);
});

test("desktopOverlayOwnsShortcuts detects both portaled dialogs and in-root #root dialogs", () => {
  class FakeDocNode {
    constructor(
      public readonly hasOverlay: boolean,
      public readonly isSelf = false,
    ) {}
    matches(): boolean {
      return this.isSelf;
    }
    querySelector(): unknown {
      return this.hasOverlay ? {} : null;
    }
    closest(): unknown {
      return null;
    }
  }
  class FakeDocument {
    readonly activeElement = null;
    readonly body: { children: FakeDocNode[] };
    constructor(
      private readonly appRoot: FakeDocNode,
      portals: FakeDocNode[] = [],
    ) {
      this.body = { children: [appRoot, ...portals] };
    }
    getElementById(id: string): FakeDocNode | null {
      return id === "root" ? this.appRoot : null;
    }
    querySelector(): unknown {
      return null;
    }
  }
  const g = globalThis as Record<string, unknown>;
  const savedDoc = g.Document;
  const savedEl = g.Element;
  g.Document = FakeDocument;
  g.Element = FakeDocNode;
  try {
    assertEquals(
      desktopOverlayOwnsShortcuts(new FakeDocument(new FakeDocNode(false)) as unknown as Document),
      false,
    );
    // In-root dialog inside #root (F3: narrow Desktop DeleteSessionShell / ObsidianSheet)
    assertEquals(
      desktopOverlayOwnsShortcuts(new FakeDocument(new FakeDocNode(true)) as unknown as Document),
      true,
    );
    // Portaled dialog directly under document.body
    assertEquals(
      desktopOverlayOwnsShortcuts(
        new FakeDocument(new FakeDocNode(false), [new FakeDocNode(false, true)]) as unknown as Document,
      ),
      true,
    );
  } finally {
    g.Document = savedDoc;
    g.Element = savedEl;
  }
});
