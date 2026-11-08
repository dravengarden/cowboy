// Client-side table sort (docs/widgets.md §7.5): a header toggles ascending,
// descending, then source order. Keys follow the column format, so numbers,
// sizes and dates compare as values rather than text. Empty cells always sort
// last, and equal keys keep source order.

import type { Cell, Column } from "../../../contracts/widget-v1.ts";

export type SortState = { column: number; direction: "asc" | "desc" } | null;

/** The state after activating `column`'s header. */
export function nextSort(current: SortState, column: number): SortState {
  if (current?.column !== column) return { column, direction: "asc" };
  return current.direction === "asc" ? { column, direction: "desc" } : null;
}

type Key = { rank: 0; n: number } | { rank: 1; s: string } | { rank: 2 };

const NUMERIC_FORMATS = new Set(["number", "percent", "bytes", "duration"]);

export function sortKey(cell: Cell | undefined, format: Column["format"]): Key {
  const value = typeof cell === "object" && cell !== null ? cell.v : cell;
  if (value === null || value === undefined || value === "") return { rank: 2 };
  if (typeof value === "boolean") return { rank: 0, n: value ? 1 : 0 };
  if (typeof value === "number") return Number.isFinite(value) ? { rank: 0, n: value } : { rank: 2 };
  if (format === "date") {
    const ms = Date.parse(value);
    if (Number.isFinite(ms)) return { rank: 0, n: ms };
  } else if (format && NUMERIC_FORMATS.has(format)) {
    const n = Number(value.trim());
    if (value.trim() !== "" && Number.isFinite(n)) return { rank: 0, n };
  }
  return { rank: 1, s: value };
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function compareKeys(a: Key, b: Key): number {
  if (a.rank !== b.rank) return a.rank - b.rank;
  if (a.rank === 0 && b.rank === 0) return a.n - b.n;
  if (a.rank === 1 && b.rank === 1) return collator.compare(a.s, b.s);
  return 0;
}

/** Row indexes in display order. */
export function sortedRowIndexes(
  rows: readonly (readonly Cell[])[],
  sort: SortState,
  format: Column["format"],
): number[] {
  const indexes = rows.map((_, index) => index);
  if (!sort) return indexes;
  const keys = rows.map((row) => sortKey(row[sort.column], format));
  const sign = sort.direction === "asc" ? 1 : -1;
  return indexes.sort((a, b) => {
    const ka = keys[a]!;
    const kb = keys[b]!;
    // Empty cells stay last in both directions.
    if ((ka.rank === 2) !== (kb.rank === 2)) return ka.rank === 2 ? 1 : -1;
    return sign * compareKeys(ka, kb) || a - b;
  });
}
