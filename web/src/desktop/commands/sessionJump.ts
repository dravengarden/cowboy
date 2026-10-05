import { useSyncExternalStore } from "react";
import { DESKTOP_JUMP_LABELS } from "./workspaceShortcuts";

/** Dispatched on the Sessions list with `{ label }`; the list cancels it once
 *  it has opened the labelled session. */
export const DESKTOP_SESSION_JUMP_EVENT = "cowboy:desktop-session-jump";
/** Dispatched on the Sessions list to reopen the previously open session. */
export const DESKTOP_SESSION_ALTERNATE_EVENT = "cowboy:desktop-session-alternate";

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
