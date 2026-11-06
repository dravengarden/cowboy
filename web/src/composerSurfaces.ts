// The composer's modal surfaces share one slot: opening one replaces whatever
// was open, so no handler has to remember to close the others. The full-screen
// editor and the delivery picker are not in the slot yet (their transitions
// move focus synchronously; see docs/composer-state-model.md).

import { useCallback, useReducer } from "react";
import type { SessionAction } from "./agentCommands";
import type { DraftSchedule } from "./protocol";

/** `id` names an existing scheduled draft (reschedule/cancel); `undefined`
 *  schedules the composer's current content into a fresh draft. */
export interface ScheduleTarget {
  id: string | undefined;
  initial: DraftSchedule | null;
}

export type ComposerModal =
  | { kind: "none" }
  | { kind: "schedule"; target: ScheduleTarget }
  | { kind: "context"; action: SessionAction }
  | { kind: "move"; sourceId: string }
  | { kind: "toolbarSettings" };

export type ComposerModalEvent =
  | { type: "open"; modal: Exclude<ComposerModal, { kind: "none" }> }
  | { type: "close"; kind: Exclude<ComposerModal["kind"], "none"> };

const NONE: ComposerModal = { kind: "none" };

/** A close names the surface it closes, so a late close from a replaced
 *  dialog (its exit transition) cannot dismiss its successor. */
export function composerModalReducer(
  current: ComposerModal,
  event: ComposerModalEvent,
): ComposerModal {
  if (event.type === "open") return event.modal;
  return current.kind === event.kind ? NONE : current;
}

export function useComposerModal(): {
  modal: ComposerModal;
  open: (modal: Exclude<ComposerModal, { kind: "none" }>) => void;
  close: (kind: Exclude<ComposerModal["kind"], "none">) => void;
} {
  const [modal, dispatch] = useReducer(composerModalReducer, NONE);
  const open = useCallback(
    (next: Exclude<ComposerModal, { kind: "none" }>) => dispatch({ type: "open", modal: next }),
    [],
  );
  const close = useCallback(
    (kind: Exclude<ComposerModal["kind"], "none">) => dispatch({ type: "close", kind }),
    [],
  );
  return { modal, open, close };
}
