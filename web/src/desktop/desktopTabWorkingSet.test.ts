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
  // Preserves strip order for visible tabs.
  assertEquals(res.visible.map((t) => t.key), ["s1", "s2", "s5", "s7", "s8", "s9", "s10", "s12", "s14"]);
  // Overflow starts with most recently visited excluded tabs.
  assertEquals(res.overflow.slice(0, 2).map((t) => t.key), ["s6", "s4"]);
  assertEquals(res.overflow.length, 6);
});
