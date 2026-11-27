import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { partitionDesktopTabWorkingSet } from "./desktopTabWorkingSet.ts";

function tab(key: string, pinned = false) {
  return { key, pinned };
}

test("keeps all tabs visible when within the 9-tab working set", () => {
  const items = [tab("a", true), tab("b"), tab("c"), tab("d")];
  const res = partitionDesktopTabWorkingSet(items, "c", ["c", "b"]);
  assertEquals(res.visible.map((t) => t.key), ["a", "b", "c", "d"]);
  assertEquals(res.overflow, []);
});

test("keeps pinned tabs and active tab inside the 9 visible slots and orders overflow by MRU", () => {
  const items = Array.from({ length: 15 }, (_, i) => tab(`s${i + 1}`, i < 2));
  const res = partitionDesktopTabWorkingSet(items, "s14", ["s14", "s12", "s7", "s5", "s10", "s9", "s8", "s6", "s4"]);
  assertEquals(res.visible.length, 9);
  assertEquals(res.visible.some((t) => t.key === "s1"), true);
  assertEquals(res.visible.some((t) => t.key === "s2"), true);
  assertEquals(res.visible.some((t) => t.key === "s14"), true);
  assertEquals(res.visible.map((t) => t.key), ["s1", "s2", "s5", "s7", "s8", "s9", "s10", "s12", "s14"]);
  assertEquals(res.overflow.slice(0, 2).map((t) => t.key), ["s6", "s4"]);
  assertEquals(res.overflow.length, 6);
});

test("reserves a visible slot for an unpinned active tab even when 9 or more tabs are pinned", () => {
  const items = [
    ...Array.from({ length: 9 }, (_, i) => tab(`pin-${i + 1}`, true)),
    tab("active-unpinned", false),
    tab("other-unpinned", false),
  ];
  const res = partitionDesktopTabWorkingSet(items, "active-unpinned", ["active-unpinned"]);
  assertEquals(res.visible.length, 9);
  assertEquals(res.visible.some((t) => t.key === "active-unpinned"), true);
  assertEquals(res.overflow.some((t) => t.key === "active-unpinned"), false);
  // 9th pinned tab yields its visible slot to the active tab and sits safely in overflow (pinned, so never bulk-closed).
  assertEquals(res.overflow.some((t) => t.key === "pin-9" && t.pinned), true);
  assertEquals(res.overflow.some((t) => t.key === "other-unpinned"), true);
});
