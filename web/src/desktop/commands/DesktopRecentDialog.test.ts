import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  desktopRecentAge,
  desktopRecentDigit,
  filterRecentItems,
  stepRecentIndex,
} from "./DesktopRecentDialog.tsx";

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

test("stepRecentIndex never produces -1 on an empty list and wraps cleanly", () => {
  assertEquals(stepRecentIndex(0, 0, "prev"), 0);
  assertEquals(stepRecentIndex(0, 0, "next"), 0);
  assertEquals(stepRecentIndex(0, 0, "last"), 0);
  assertEquals(stepRecentIndex(0, 0, "first"), 0);

  assertEquals(stepRecentIndex(3, 0, "prev"), 2);
  assertEquals(stepRecentIndex(3, 2, "next"), 0);
  assertEquals(stepRecentIndex(3, 1, "last"), 2);
  assertEquals(stepRecentIndex(3, 2, "first"), 0);
});

test("filterRecentItems matches multi-token queries across title, detail, and kind", () => {
  const items = [
    { key: "s1", kind: "session" as const, title: "Fix Desktop Tabs", detail: "cowboy", at: 100 },
    { key: "d1", kind: "draft" as const, title: "Release Notes", detail: "v3.144", at: 200 },
  ];
  assertEquals(filterRecentItems(items, "").length, 2);
  assertEquals(filterRecentItems(items, "desktop cowboy").map((i) => i.key), ["s1"]);
  assertEquals(filterRecentItems(items, "draft release").map((i) => i.key), ["d1"]);
  assertEquals(filterRecentItems(items, "missing"), []);
});
