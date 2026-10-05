import { useSyncExternalStore } from "react";
import { DESKTOP_JUMP_LABELS } from "./workspaceShortcuts";

/** Dispatched on the Sessions list with `{ label }`; the list cancels it once
 *  it has opened the labelled session. */
export const DESKTOP_SESSION_JUMP_EVENT = "cowboy:desktop-session-jump";

/** Dispatched on the Sessions list with `{ key, pending }` while the Move
 *  pick layer is armed; the list sets `pending` to keep the layer open
 *  (a label prefix, or a key that names no folder). */
export const DESKTOP_MOVE_PICK_EVENT = "cowboy:desktop-move-pick";

/** Pick labels in key priority, home row first. Past 26 targets every label
 *  is two letters, so no label is a prefix of another. */
export function desktopPickLabels(count: number): string[] {
  const pool = [...DESKTOP_JUMP_LABELS];
  if (count <= pool.length) return pool.slice(0, count);
  const labels: string[] = [];
  for (const first of pool) {
    for (const second of pool) {
      if (labels.length === count) return labels;
      labels.push(first + second);
    }
  }
  return labels;
}

export interface SessionJumpTarget {
  readonly label: string;
  readonly id: string;
  readonly title: string;
  readonly detail: string;
  readonly current: boolean;
}

/** Labels follow the flat displayed order, so they read top to bottom and do
 *  not change when a folder is folded. */
export function sessionJumpLabels<T>(items: readonly T[]): Map<T, string> {
  const labels = new Map<T, string>();
  items.slice(0, DESKTOP_JUMP_LABELS.length).forEach((item, index) => {
    labels.set(item, DESKTOP_JUMP_LABELS[index]!);
  });
  return labels;
}

let targets: readonly SessionJumpTarget[] = [];
const listeners = new Set<() => void>();

export function publishSessionJumpTargets(
  next: readonly SessionJumpTarget[],
): void {
  const same = next.length === targets.length &&
    next.every((target, index) => {
      const previous = targets[index]!;
      return previous.id === target.id && previous.label === target.label &&
        previous.title === target.title && previous.detail === target.detail &&
        previous.current === target.current;
    });
  if (same) return;
  targets = next;
  for (const listener of listeners) listener();
}

export function useSessionJumpTargets(): readonly SessionJumpTarget[] {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => targets,
    () => targets,
  );
}
