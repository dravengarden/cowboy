import { describe, expect, test } from "bun:test";
import { nextSort, sortedRowIndexes, sortKey } from "./widgetTableModel";
import { previewSlice } from "./widgetMode";

describe("widget table sort", () => {
  test("a header cycles ascending, descending, then source order", () => {
    const asc = nextSort(null, 1);
    expect(asc).toEqual({ column: 1, direction: "asc" });
    const desc = nextSort(asc, 1);
    expect(desc).toEqual({ column: 1, direction: "desc" });
    expect(nextSort(desc, 1)).toBeNull();
    expect(nextSort(desc, 0)).toEqual({ column: 0, direction: "asc" });
  });

  test("numbers compare as values and text naturally", () => {
    const rows = [["b10"], ["b9"], ["a"]];
    expect(sortedRowIndexes(rows, { column: 0, direction: "asc" }, undefined)).toEqual([2, 1, 0]);
    const sizes = [[2048], [512], [{ v: 100000 }]];
    expect(sortedRowIndexes(sizes, { column: 0, direction: "desc" }, "bytes")).toEqual([2, 0, 1]);
    expect(sortKey("12.5", "number")).toEqual({ rank: 0, n: 12.5 });
    expect(sortKey("12.5", "text")).toEqual({ rank: 1, s: "12.5" });
  });

  test("dates compare by time, and empty cells stay last both ways", () => {
    const rows = [["2026-11-08T10:00:00Z"], [null], ["2026-01-01T00:00:00Z"], [""]];
    expect(sortedRowIndexes(rows, { column: 0, direction: "asc" }, "date")).toEqual([2, 0, 1, 3]);
    expect(sortedRowIndexes(rows, { column: 0, direction: "desc" }, "date")).toEqual([0, 2, 1, 3]);
  });

  test("equal keys keep source order", () => {
    const rows = [["x", 1], ["x", 2], ["x", 3]];
    expect(sortedRowIndexes(rows, { column: 0, direction: "desc" }, undefined)).toEqual([0, 1, 2]);
    expect(sortedRowIndexes(rows, null, undefined)).toEqual([0, 1, 2]);
  });

  test("the preview bounds a collection; the inspector draws it all", () => {
    const items = Array.from({ length: 55 }, (_, i) => i);
    expect(previewSlice(items, 50, "preview").hidden).toBe(5);
    expect(previewSlice(items, 50, "preview").shown).toHaveLength(50);
    expect(previewSlice(items, 50, "inspector")).toEqual({ shown: items, hidden: 0 });
  });
});
