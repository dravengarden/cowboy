// Per-device Desktop diff preferences: unified or split (`␣VD`), and the
// patch's own context or the whole file (`␣VZ`).
import { useSyncExternalStore } from "react";

function devicePreference<T extends string>(storageKey: string, initial: T, other: T) {
  const listeners = new Set<() => void>();
  let value: T | undefined;
  const current = (): T => {
    if (value) return value;
    try {
      value = globalThis.localStorage?.getItem(storageKey) === other ? other : initial;
    } catch {
      value = initial;
    }
    return value;
  };
  const toggle = (): T => {
    value = current() === other ? initial : other;
    try {
      globalThis.localStorage?.setItem(storageKey, value);
    } catch {
      // The choice then lasts for this page only.
    }
    for (const listener of listeners) listener();
    return value;
  };
  const usePreference = (): T =>
    useSyncExternalStore(
      (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      current,
      () => initial,
    );
  return { toggle, usePreference };
}

export type ReviewDiffLayout = "unified" | "split";
const layout = devicePreference<ReviewDiffLayout>("cowboy.desktop.reviewDiffLayout.v1", "unified", "split");
export const toggleReviewDiffLayout = layout.toggle;
export const useReviewDiffLayout = layout.usePreference;

export type ReviewDiffContext = "patch" | "file";
const context = devicePreference<ReviewDiffContext>("cowboy.desktop.reviewDiffContext.v1", "patch", "file");
export const toggleReviewDiffContext = context.toggle;
export const useReviewDiffContext = context.usePreference;
