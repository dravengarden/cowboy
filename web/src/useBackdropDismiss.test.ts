import { assertEquals } from "jsr:@std/assert";
import { shouldBlockBackdropClick } from "./backdropDismiss.ts";

const guard = { x: 320, y: 640, expiresAt: 1_700 };

Deno.test("backdrop guard consumes the paired compatibility click", () => {
  assertEquals(
    shouldBlockBackdropClick(
      guard,
      { clientX: 320, clientY: 640, detail: 1 },
      1_200,
    ),
    true,
  );
  assertEquals(
    shouldBlockBackdropClick(
      guard,
      { clientX: 334, clientY: 628, detail: 1 },
      1_200,
    ),
    true,
  );
  assertEquals(
    shouldBlockBackdropClick(
      guard,
      { clientX: 320, clientY: 640, detail: 0, pointerType: "touch" },
      1_200,
    ),
    true,
  );
});

Deno.test("backdrop guard preserves keyboard, later, and unrelated clicks", () => {
  assertEquals(
    shouldBlockBackdropClick(
      guard,
      { clientX: 320, clientY: 640, detail: 0 },
      1_200,
    ),
    false,
  );
  assertEquals(
    shouldBlockBackdropClick(
      guard,
      { clientX: 320, clientY: 640, detail: 1 },
      1_701,
    ),
    false,
  );
  assertEquals(
    shouldBlockBackdropClick(
      guard,
      { clientX: 420, clientY: 640, detail: 1 },
      1_200,
    ),
    false,
  );
});
