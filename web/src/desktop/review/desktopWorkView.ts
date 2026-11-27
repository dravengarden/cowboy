// Desktop's work pane shows one of three views of the open Session: its
// Conversation, its code Review (docs/desktop-code-review.md), or its
// Background Tasks. All mounted views stay alive behind the tabs; this module
// owns which one is shown. Review preference is per Session and per device.

import { useSyncExternalStore } from "react";

export type DesktopWorkView = "conversation" | "review" | "tasks";

const STORAGE_KEY = "cowboy.desktop.workView.v1";
/** Sessions remembered; the least recently changed are forgotten first. */
export const DESKTOP_WORK_VIEW_LIMIT = 100;

function readViews(): string[] {
  try {
    const parsed: unknown = JSON.parse(
      globalThis.localStorage?.getItem(STORAGE_KEY) ?? "[]",
    );
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return [];
  }
}

/** Sessions whose stored view is Review, oldest change first. Conversation is
 *  the default and is stored as absence. */
export function storedReviewSessions(
  stored: readonly string[],
  sessionId: string,
  view: DesktopWorkView,
): string[] {
  const kept = stored.filter((id) => id !== sessionId);
  if (view === "review") kept.push(sessionId);
  return kept.slice(-DESKTOP_WORK_VIEW_LIMIT);
}

export function loadDesktopWorkView(sessionId: string): DesktopWorkView {
  return readViews().includes(sessionId) ? "review" : "conversation";
}

export function saveDesktopWorkView(sessionId: string, view: DesktopWorkView): void {
  try {
    globalThis.localStorage?.setItem(
      STORAGE_KEY,
      JSON.stringify(storedReviewSessions(readViews(), sessionId, view)),
    );
  } catch {
    // A full or blocked storage only loses the remembered view.
  }
}

// What the mounted workspace shows now, for commands and the status line that
// live outside it (Follow, History/Explore, Page Index answer only to the
// transcript).
let shown: DesktopWorkView = "conversation";
const listeners = new Set<() => void>();

export function publishDesktopWorkView(view: DesktopWorkView): void {
  if (view === shown) return;
  shown = view;
  for (const listener of listeners) listener();
}

export function desktopWorkView(): DesktopWorkView {
  return shown;
}

export function desktopConversationShown(): boolean {
  return shown === "conversation";
}

export function desktopReviewShown(): boolean {
  return shown === "review";
}

export function desktopTasksShown(): boolean {
  return shown === "tasks";
}

export function useDesktopWorkViewState(): DesktopWorkView {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    desktopWorkView,
    () => "conversation",
  );
}

export function useDesktopReviewShown(): boolean {
  return useDesktopWorkViewState() === "review";
}

export function useDesktopTasksShown(): boolean {
  return useDesktopWorkViewState() === "tasks";
}

/** Ask the mounted workspace to show a view (`␣C` returns to Conversation,
 *  `␣V` shows Review, `␣G` shows Tasks). */
export const DESKTOP_WORK_VIEW_EVENT = "cowboy:desktop-work-view";

export function requestDesktopWorkView(view: DesktopWorkView): void {
  globalThis.dispatchEvent(
    new CustomEvent<DesktopWorkView>(DESKTOP_WORK_VIEW_EVENT, { detail: view }),
  );
}

export function desktopWorkViewFromEvent(event: Event): DesktopWorkView | null {
  const detail = (event as CustomEvent<unknown>).detail;
  return detail === "conversation" || detail === "review" || detail === "tasks" ? detail : null;
}

// An agent's or a link's reveal can arrive before the workbench has mounted:
// the work-view hook marks it pending and the workbench takes it on mount or
// at once when it is already there.
let revealPending = false;
export const DESKTOP_REVIEW_REVEAL_EVENT = "cowboy:desktop-review-reveal";

export function requestDesktopReviewReveal(): void {
  revealPending = true;
  globalThis.dispatchEvent(new Event(DESKTOP_REVIEW_REVEAL_EVENT));
}

export function takeDesktopReviewReveal(): boolean {
  const pending = revealPending;
  revealPending = false;
  return pending;
}
