import { expect, test } from "bun:test";
import { clampMobilePillPosition } from "./MessageActions.tsx";
import { pickViewportMessageCopyId, rectIntersectsViewport } from "./desktop/transcriptYank.ts";
import {
  closeMobileMessageActions,
  getMobileMessageActions,
  invokeRegisteredMessageCopy,
  openMobileMessageActions,
  registerMessageCopyHandler,
} from "./messageCopyRegistry.ts";

test("messageCopyRegistry registers, dispatches, and cleans up handlers safely", async () => {
  const modes: string[] = [];
  const unregister = registerMessageCopyHandler("sess-1:10", async (mode) => {
    modes.push(mode);
    return true;
  });
  expect(await invokeRegisteredMessageCopy("sess-1:10", "rich")).toBe(true);
  expect(await invokeRegisteredMessageCopy("sess-1:10", "markdown")).toBe(true);
  expect(modes).toEqual(["rich", "markdown"]);
  unregister();
  expect(await invokeRegisteredMessageCopy("sess-1:10", "rich")).toBe(false);
});

test("mobile floating pill store keeps at most one active message at a time", () => {
  closeMobileMessageActions();
  expect(getMobileMessageActions()).toBeNull();
  openMobileMessageActions("s1:1", 120, 300);
  expect(getMobileMessageActions()).toEqual({ copyId: "s1:1", x: 120, y: 300 });
  openMobileMessageActions("s1:2", 180, 420);
  expect(getMobileMessageActions()).toEqual({ copyId: "s1:2", x: 180, y: 420 });
  closeMobileMessageActions("s1:1"); // wrong id does not close s1:2
  expect(getMobileMessageActions()?.copyId).toBe("s1:2");
  closeMobileMessageActions("s1:2");
  expect(getMobileMessageActions()).toBeNull();
});

test("clampMobilePillPosition keeps floating pill inside viewport margins and away from top/bottom chrome", () => {
  // Normal mid-screen tap places pill cleanly above thumb
  const mid = clampMobilePillPosition(200, 400, 390, 844);
  expect(mid.left).toBeGreaterThanOrEqual(12);
  expect(mid.left + 228).toBeLessThanOrEqual(390 - 12);
  expect(mid.top).toBeLessThan(400);

  // Near top edge flips below tap so header is never overlapped
  const topEdge = clampMobilePillPosition(20, 70, 390, 844);
  expect(topEdge.left).toBe(12);
  expect(topEdge.top).toBeGreaterThan(70);

  // Near right & bottom edge stays strictly inside viewport above composer
  const bottomRight = clampMobilePillPosition(385, 830, 390, 844);
  expect(bottomRight.left + 228).toBeLessThanOrEqual(390 - 12);
  expect(bottomRight.top + 44).toBeLessThanOrEqual(844 - 76);
});

test("pickViewportMessageCopyId selects tall spanning reply first, then nearest visible assistant reply, then user fallback", () => {
  // Tall assistant message spanning the whole viewport (-300..1200 in 0..800 viewport)
  expect(
    pickViewportMessageCopyId(
      [
        { copyId: "u1", role: "user", top: -450, bottom: -320 },
        { copyId: "a-tall", role: "assistant", top: -300, bottom: 1200 },
      ],
      0,
      800,
    ),
  ).toBe("a-tall");

  // Two short messages visible, neither spans focal line (360): prefers nearest assistant
  expect(
    pickViewportMessageCopyId(
      [
        { copyId: "u2", role: "user", top: 200, bottom: 320 },
        { copyId: "a2", role: "assistant", top: 410, bottom: 520 },
      ],
      0,
      800,
    ),
  ).toBe("a2");

  // Only user bubble visible: falls back cleanly to user bubble
  expect(
    pickViewportMessageCopyId(
      [{ copyId: "u3", role: "user", top: 100, bottom: 200 }],
      0,
      800,
    ),
  ).toBe("u3");

  // Off-screen messages ignored
  expect(
    pickViewportMessageCopyId(
      [{ copyId: "off", role: "assistant", top: 900, bottom: 1100 }],
      0,
      800,
    ),
  ).toBeNull();
});

test("clampMobilePillPosition respects measured pill size and zoomed visualViewport offsets", () => {
  const pos = clampMobilePillPosition(350, 260, 320, 500, 240, 48, 80, 120);
  expect(pos.left).toBeGreaterThanOrEqual(80 + 12);
  expect(pos.left + 240).toBeLessThanOrEqual(80 + 320 - 12);
  expect(pos.top).toBeGreaterThanOrEqual(120 + 56);
  expect(pos.top + 48).toBeLessThanOrEqual(120 + 500 - 76);
});

test("rectIntersectsViewport rejects scrolled-offscreen focused message wrappers", () => {
  expect(rectIntersectsViewport(-200, -10, 0, 700)).toBe(false);
  expect(rectIntersectsViewport(710, 900, 0, 700)).toBe(false);
  expect(rectIntersectsViewport(-50, 120, 0, 700)).toBe(true);
});

test("clampMobilePillPosition keeps pill inside narrow and short zoomed visual viewports", () => {
  // Very narrow zoomed viewport (180x220) at offset (50, 80)
  const narrow = clampMobilePillPosition(200, 120, 180, 220, 228, 44, 50, 80);
  expect(narrow.left).toBeGreaterThanOrEqual(50);
  expect(narrow.left).toBeLessThanOrEqual(50 + 180 - 8);
  expect(narrow.top).toBeGreaterThanOrEqual(80);
  expect(narrow.top + 44).toBeLessThanOrEqual(80 + 220);
});
