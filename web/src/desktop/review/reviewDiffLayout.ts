// Unified or split diffs (`␣VD`), a per-device Desktop preference.
import { useSyncExternalStore } from "react";

export type ReviewDiffLayout = "unified" | "split";
const STORAGE_KEY = "cowboy.desktop.reviewDiffLayout.v1";
const listeners = new Set<() => void>();
let layout: ReviewDiffLayout | undefined;

function current(): ReviewDiffLayout {
  if (layout) return layout;
  try {
    layout = globalThis.localStorage?.getItem(STORAGE_KEY) === "split" ? "split" : "unified";
  } catch {
    layout = "unified";
  }
  return layout;
}

export function toggleReviewDiffLayout(): ReviewDiffLayout {
  layout = current() === "split" ? "unified" : "split";
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, layout);
  } catch {
    // The choice then lasts for this page only.
  }
  for (const listener of listeners) listener();
  return layout;
}

export function useReviewDiffLayout(): ReviewDiffLayout {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    current,
    () => "unified",
  );
}
