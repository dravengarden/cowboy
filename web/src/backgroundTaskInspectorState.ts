// Which background task's details are open, and where focus returns after.
// One inspector per session; the transcript rows, the bar and `␣G` open it.

import { useSyncExternalStore } from "react";

export type InspectorOrigin = "row" | "bar" | "command" | "link";

export interface InspectorState {
  sessionId: string | null;
  /** The selected task; null shows the list (Mobile) or the first task. */
  key: string | null;
  origin: InspectorOrigin;
  /** Restored on close: the originating row, or the composer from the bar. */
  returnFocus: HTMLElement | null;
  /** Bumped on every open so a second open of the same key re-selects it. */
  revision: number;
}

let current: InspectorState = {
  sessionId: null,
  key: null,
  origin: "row",
  returnFocus: null,
  revision: 0,
};
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

export function inspectorState(): InspectorState {
  return current;
}

export function openBackgroundTask(
  sessionId: string,
  key: string | null,
  origin: InspectorOrigin,
  returnFocus: HTMLElement | null = null,
): void {
  const active = globalThis.document?.activeElement;
  current = {
    sessionId,
    key,
    origin,
    returnFocus: returnFocus ?? (active instanceof HTMLElement ? active : null),
    revision: current.revision + 1,
  };
  emit();
}

/** Select another task inside an open inspector (stable by key). */
export function selectBackgroundTask(key: string | null): void {
  if (current.sessionId === null || current.key === key) return;
  current = { ...current, key };
  emit();
}

export function closeBackgroundTasks(): HTMLElement | null {
  if (current.sessionId === null) return null;
  const focus = current.returnFocus;
  current = { ...current, sessionId: null, key: null, returnFocus: null };
  emit();
  return focus;
}

export function useInspectorState(): InspectorState {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return (): void => {
        listeners.delete(listener);
      };
    },
    () => current,
    () => current,
  );
}

/** Window event the `␣G` command dispatches; the active inspector host opens. */
export const OPEN_BACKGROUND_TASKS_EVENT = "cowboy:open-background-tasks";
