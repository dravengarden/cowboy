import { desktopImeOwnsKey } from "./commands/imeShortcut";
import { workspaceCommandKey } from "./commands/workspaceCommandKey";
import { getVimMode } from "../vimModeStore";
import { vimSinkAwaitsInput } from "./vim/vimSinkInput";

export type PromptChordKind = "[" | "]" | "Z";

export interface PromptVimAction {
  kind:
    | "recall-last-pending"
    | "cycle-send-order-prev"
    | "cycle-send-order-next"
    | "save-and-close"
    | "discard-and-close";
}

export interface PromptChordTracker {
  prefix: PromptChordKind | null;
  timer: ReturnType<typeof setTimeout> | null;
}

export function createPromptChordTracker(): PromptChordTracker {
  return { prefix: null, timer: null };
}

export function clearPromptChord(tracker: PromptChordTracker): void {
  if (tracker.timer !== null) {
    globalThis.clearTimeout(tracker.timer);
    tracker.timer = null;
  }
  tracker.prefix = null;
}

function armPromptChord(
  tracker: PromptChordTracker,
  prefix: PromptChordKind,
): void {
  clearPromptChord(tracker);
  tracker.prefix = prefix;
  tracker.timer = globalThis.setTimeout(() => {
    tracker.prefix = null;
    tracker.timer = null;
  }, 900);
}

/**
 * Evaluate Desktop Prompt canvas Vim-native sequences & empty-prompt recall:
 * - Empty New Prompt `↑` (or Vim Normal `k` with no pending operator): recall
 *   the last queued/drafted message into Insert at end.
 * - Vim Normal `[q` / `]q`: walk send order (Queue items ↔ New Prompt).
 * - Vim Normal `ZZ` / `ZQ` (on Queue/Draft buffer): save-and-close / discard.
 */
export function evaluatePromptCanvasKey({
  event,
  tracker,
  isNewPrompt,
  isEmpty,
  hasCompletionOpen,
  canRecallLast,
  canCycleQueue,
}: {
  event: KeyboardEvent;
  tracker: PromptChordTracker;
  isNewPrompt: boolean;
  isEmpty: boolean;
  hasCompletionOpen: boolean;
  canRecallLast: boolean;
  canCycleQueue: boolean;
}): { consumed: boolean; action?: PromptVimAction } {
  if (desktopImeOwnsKey(event)) return { consumed: false };
  if (event.metaKey || event.ctrlKey || event.altKey) {
    clearPromptChord(tracker);
    return { consumed: false };
  }
  const target = event.target && typeof (event.target as Element).matches === "function"
    ? (event.target as Element)
    : null;
  const onSink = Boolean(target?.matches("[data-vim-command-sink]"));
  const awaitsInput = typeof Element !== "undefined" && vimSinkAwaitsInput(target);
  const inPlainNormal = onSink && getVimMode() === "normal" && !awaitsInput;

  // Empty New Prompt recall: ↑ in Insert/non-Vim, or k in plain Normal.
  if (isNewPrompt && isEmpty && !hasCompletionOpen && canRecallLast) {
    const isRecallKey = (!event.shiftKey && event.key === "ArrowUp") ||
      (inPlainNormal && !event.shiftKey && event.code === "KeyK");
    if (isRecallKey) {
      clearPromptChord(tracker);
      return { consumed: true, action: { kind: "recall-last-pending" } };
    }
  }

  if (!inPlainNormal) {
    clearPromptChord(tracker);
    return { consumed: false };
  }

  const key = workspaceCommandKey(event);
  if (tracker.prefix !== null) {
    const prefix = tracker.prefix;
    clearPromptChord(tracker);
    if (event.key === "Escape") {
      return { consumed: true };
    }
    if (prefix === "[" && key === "q" && canCycleQueue) {
      return { consumed: true, action: { kind: "cycle-send-order-prev" } };
    }
    if (prefix === "]" && key === "q" && canCycleQueue) {
      return { consumed: true, action: { kind: "cycle-send-order-next" } };
    }
    if (prefix === "Z" && !isNewPrompt && key === "Z") {
      return { consumed: true, action: { kind: "save-and-close" } };
    }
    if (prefix === "Z" && !isNewPrompt && key === "Q") {
      return { consumed: true, action: { kind: "discard-and-close" } };
    }
    return { consumed: false };
  }

  if ((key === "[" || key === "]") && canCycleQueue) {
    armPromptChord(tracker, key);
    return { consumed: true };
  }
  if (key === "Z" && !isNewPrompt) {
    armPromptChord(tracker, "Z");
    return { consumed: true };
  }
  return { consumed: false };
}
