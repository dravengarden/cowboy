import { test } from "bun:test";
import { assertEquals } from "@std/assert";

test("the keyboard-free height is learned at startup, never from a focused, shrunken viewport", async () => {
  const globals = globalThis as unknown as Record<string, unknown>;
  const saved = ["visualViewport", "innerHeight", "document", "Element"].map((key) => [key, globals[key]] as const);
  class StubElement {
    matches(): boolean {
      return true;
    }
    closest(): null {
      return null;
    }
  }
  const viewport = { height: 844, offsetTop: 0, addEventListener() {}, removeEventListener() {} };
  const document = { activeElement: null as unknown };
  try {
    Object.assign(globals, { visualViewport: viewport, innerHeight: 844, document, Element: StubElement });
    const fresh = await import(`./keyboardInset.ts?startup=${Date.now()}`);
    assertEquals(fresh.keyboardFreeHeightForTest(), 844);

    // Evaluated while a field is focused and the native shell has already
    // shrunk the viewport: that keyboard-sized height is never the baseline.
    document.activeElement = new StubElement();
    Object.assign(globals, { innerHeight: 430 });
    viewport.height = 430;
    const focused = await import(`./keyboardInset.ts?focused=${Date.now()}`);
    assertEquals(focused.keyboardFreeHeightForTest(), 0);
  } finally {
    for (const [key, value] of saved) {
      if (value === undefined) delete globals[key];
      else globals[key] = value;
    }
  }
});
