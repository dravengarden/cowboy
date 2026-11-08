// The composer's transient surfaces share one slot: opening one replaces whatever
// was open, so no handler has to remember to close the others. The full-screen
// editor is orthogonal to the slot (the delivery picker opens from its send
// button), so it is a separate flag in the same reducer. Expand and collapse
// move the iOS first responder: callers dispatch them inside `flushSync` within
// the user gesture, never from an effect (docs/composer-state-model.md).

import { useCallback, useMemo, useReducer } from "react";
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
  | { kind: "toolbarSettings" }
  | { kind: "stop" }
  | { kind: "delivery"; anchor: HTMLElement }
  /** Deliberately a non-modal Popper: it leaves the iOS first responder alone. */
  | { kind: "clear"; anchor: HTMLElement };

type OpenModal = Exclude<ComposerModal, { kind: "none" }>;
type ModalKind = OpenModal["kind"];

export interface ComposerSurfaces {
  fullscreen: boolean;
  modal: ComposerModal;
}

export type ComposerSurfaceEvent =
  | { type: "open"; modal: OpenModal }
  | { type: "close"; kind: ModalKind }
  | { type: "fullscreen"; open: boolean }
  /** A context reset ends the input interaction on touch devices: the
   *  full-screen editor and the clear-draft popper go with it. */
  | { type: "contextReset" };

const NONE: ComposerModal = { kind: "none" };
const INITIAL: ComposerSurfaces = { fullscreen: false, modal: NONE };

/** A close names the surface it closes, so a late close from a replaced
 *  dialog (its exit transition) cannot dismiss its successor. */
export function composerSurfacesReducer(
  current: ComposerSurfaces,
  event: ComposerSurfaceEvent,
): ComposerSurfaces {
  switch (event.type) {
    case "open":
      return { ...current, modal: event.modal };
    case "close":
      return current.modal.kind === event.kind ? { ...current, modal: NONE } : current;
    case "fullscreen":
      return current.fullscreen === event.open ? current : { ...current, fullscreen: event.open };
    case "contextReset":
      return {
        fullscreen: false,
        modal: current.modal.kind === "clear" ? NONE : current.modal,
      };
  }
}

export function useComposerSurfaces(): ComposerSurfaces & {
  open: (modal: OpenModal) => void;
  close: (kind: ModalKind) => void;
  setFullscreen: (open: boolean) => void;
  contextReset: () => void;
} {
  const [state, dispatch] = useReducer(composerSurfacesReducer, INITIAL);
  const open = useCallback((modal: OpenModal) => dispatch({ type: "open", modal }), []);
  const close = useCallback((kind: ModalKind) => dispatch({ type: "close", kind }), []);
  const setFullscreen = useCallback((next: boolean) => dispatch({ type: "fullscreen", open: next }), []);
  const contextReset = useCallback(() => dispatch({ type: "contextReset" }), []);
  return useMemo(
    () => ({ ...state, open, close, setFullscreen, contextReset }),
    [state, open, close, setFullscreen, contextReset],
  );
}
