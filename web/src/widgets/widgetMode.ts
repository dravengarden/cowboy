// Where a widget is drawn (docs/widgets.md §7.1): the transcript preview
// bounds long collections independently of the height fold; the inspector
// draws everything and pages long tables.

import { createContext, useContext, useState } from "react";
import type { SortState } from "./widgetTableModel";

export type WidgetMode = "preview" | "inspector";

export const WidgetModeContext = createContext<WidgetMode>("preview");

export function useWidgetMode(): WidgetMode {
  return useContext(WidgetModeContext);
}

/** Table sort shared by one widget's row and its inspector, keyed by the
 *  parsed table node (both views draw the same document object). */
export type WidgetSorts = {
  get: (node: object) => SortState;
  set: (node: object, sort: SortState) => void;
};

export const WidgetSortContext = createContext<WidgetSorts | null>(null);

export function useWidgetSorts(): WidgetSorts {
  const [sorts, setSorts] = useState<ReadonlyMap<object, SortState>>(() => new Map());
  return {
    get: (node) => sorts.get(node) ?? null,
    set: (node, sort) =>
      setSorts((current) => new Map(current).set(node, sort)),
  };
}

/** The entries a mode draws, and how many the preview left out. */
export function previewSlice<T>(
  items: readonly T[],
  limit: number,
  mode: WidgetMode,
): { shown: readonly T[]; hidden: number } {
  if (mode === "inspector" || items.length <= limit) return { shown: items, hidden: 0 };
  return { shown: items.slice(0, limit), hidden: items.length - limit };
}

export function moreLabel(hidden: number, noun: string): string {
  return `${String(hidden)} more ${noun}${hidden === 1 ? "" : "s"} · open to see all`;
}
