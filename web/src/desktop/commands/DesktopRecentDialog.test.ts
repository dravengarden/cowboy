import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { desktopRecentAge, desktopRecentDigit } from "./DesktopRecentDialog.tsx";

test("Recent digits use the physical top row and keypad", () => {
  assertEquals(desktopRecentDigit("Digit1"), 1);
  assertEquals(desktopRecentDigit("Numpad9"), 9);
  assertEquals(desktopRecentDigit("Digit0"), null);
  assertEquals(desktopRecentDigit("KeyJ"), null);
});

test("Recent ages are short", () => {
  const now = 10 * 24 * 3_600_000;
  assertEquals(desktopRecentAge(now - 10_000, now), "now");
  assertEquals(desktopRecentAge(now - 5 * 60_000, now), "5m");
  assertEquals(desktopRecentAge(now - 3 * 3_600_000, now), "3h");
  assertEquals(desktopRecentAge(now - 2 * 86_400_000, now), "2d");
});
