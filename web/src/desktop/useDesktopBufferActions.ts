import { type MutableRefObject, type RefObject, useCallback, useState } from "react";
import type { DeliveryTarget } from "../currentTurnDelivery";
import { moveSessionDraftToDocument } from "../documents/sessionDraftImport";
import type { Delivery, Status } from "../protocol";
import {
  activateDraft,
  currentTurnPrompt,
  frontPrompt,
  notify,
  pauseQueueConfirmed,
  type QueuedMessage,
  queuedToDraft,
  reorderQueue,
  submitPrompt,
  waitForPendingEdit,
} from "../store";
import type { DesktopPendingBufferHandle } from "./DesktopPendingBufferEditor";
import type { DesktopPromptBuffer } from "./promptBuffer";

export interface NewPromptCallbacks {
  sendable: boolean;
  canInsert: boolean;
  canJumpFront: boolean;
  canForce: boolean;
  canCurrentTurn: boolean;
  pending: boolean;
  progress: boolean;
  sendLabel: "Send" | "Queue" | "Send…" | "Enqueue";
  sendDescription: string;
  unavailableReason: string;
  onAttach: () => void;
  onSaveDraft: () => void;
  onSchedule: () => void;
  onJumpFront: () => void;
  onForce: () => void;
  onCurrentTurn: () => void;
  onSubmit: () => void;
  getText: () => string;
}

export interface DesktopBufferActionsArgs {
  sessionId: string;
  status: Status;
  paused: boolean;
  serverConnected: boolean;
  queue: readonly QueuedMessage[];
  activeBuffer: DesktopPromptBuffer;
  resolvedRow: QueuedMessage | null;
  pendingRef: RefObject<DesktopPendingBufferHandle | null>;
  sendButtonRef: RefObject<HTMLButtonElement | null>;
  bufferMeta: {
    sendable: boolean;
    dirty: boolean;
    saving: boolean;
    scheduled: boolean;
  };
  newPrompt: NewPromptCallbacks;
  onScheduleDraft: (id: string) => void;
  onMoveDraft?: ((id: string) => void) | undefined;
  onCloseBuffer: (focusTarget?: { kind: "queued" | "draft"; id: string }) => void;
  onOpenStop: () => void;
  closingRef: MutableRefObject<boolean>;
}

async function saveAndConfirmRow(
  sessionId: string,
  handle: DesktopPendingBufferHandle,
  row: QueuedMessage | null,
): Promise<boolean> {
  if (!(await handle.save({ silent: true }))) return false;
  if (row && !row.id.startsWith("opt-")) {
    await waitForPendingEdit(sessionId, row.id);
  }
  return true;
}

function buildToolbarPresentation(
  args: DesktopBufferActionsArgs,
  busy: boolean,
) {
  const { activeBuffer, resolvedRow, bufferMeta, queue, newPrompt } = args;
  if (activeBuffer.kind === "new") {
    return {
      sendLabel: newPrompt.sendLabel,
      sendIconKind: undefined as "check" | "send" | undefined,
      sendDescription: newPrompt.sendDescription,
      saveLabel: "Draft",
      saveTitle: "Save as draft",
      scheduleLabel: "Schedule",
      scheduleTitle: "Schedule send",
      canSave: newPrompt.sendable,
      canSchedule: true,
      canJumpFront: newPrompt.canJumpFront,
      canForce: newPrompt.canForce,
      sendable: newPrompt.sendable,
      pending: newPrompt.pending,
      progress: newPrompt.progress,
    };
  }
  if (activeBuffer.kind === "queued") {
    return {
      sendLabel: "Finish" as const,
      sendIconKind: "check" as const,
      sendDescription: "Save changes, preserve queue position, and release hold",
      saveLabel: "Save",
      saveTitle: "Save changes and keep queue hold",
      scheduleLabel: "Schedule",
      scheduleTitle: "Move to scheduled drafts",
      canSave: bufferMeta.sendable,
      canSchedule: Boolean(resolvedRow),
      canJumpFront: queue.length > 1,
      canForce: true,
      sendable: bufferMeta.sendable,
      pending: bufferMeta.saving,
      progress: bufferMeta.saving,
    };
  }
  return {
    sendLabel: (bufferMeta.scheduled
      ? "Deliver…"
      : busy || queue.length > 0
      ? "Send…"
      : "Send") as "Send" | "Send…" | "Deliver…",
    sendIconKind: "send" as const,
    sendDescription: bufferMeta.scheduled
      ? "Choose delivery instead of scheduled time"
      : busy || queue.length > 0
      ? "Save draft and choose delivery mode"
      : "Save and send draft",
    saveLabel: "Save",
    saveTitle: bufferMeta.scheduled ? "Save scheduled text" : "Save draft now",
    scheduleLabel: bufferMeta.scheduled ? "Reschedule" : "Schedule",
    scheduleTitle: bufferMeta.scheduled ? "Change or cancel schedule" : "Schedule draft",
    canSave: bufferMeta.sendable,
    canSchedule: Boolean(resolvedRow),
    canJumpFront: queue.length > 0,
    canForce: true,
    sendable: bufferMeta.sendable,
    pending: bufferMeta.saving,
    progress: bufferMeta.saving,
  };
}

function useBufferTransferActions(
  args: DesktopBufferActionsArgs,
  closeTarget: () => { kind: "queued" | "draft"; id: string } | undefined,
) {
  const {
    sessionId,
    queue,
    activeBuffer,
    resolvedRow,
    pendingRef,
    newPrompt,
    onScheduleDraft,
    onMoveDraft,
    onCloseBuffer,
    closingRef,
  } = args;
  const isNew = activeBuffer.kind === "new";

  const handleSchedule = useCallback((): void => {
    if (isNew) {
      newPrompt.onSchedule();
      return;
    }
    const handle = pendingRef.current;
    if (!handle || !handle.isSendable() || !resolvedRow) return;
    if (activeBuffer.kind === "queued") {
      closingRef.current = true;
      void saveAndConfirmRow(sessionId, handle, resolvedRow).then(async (ok) => {
        if (!ok) return;
        await queuedToDraft(sessionId, resolvedRow.id);
        onCloseBuffer();
        onScheduleDraft(resolvedRow.id);
      }).catch((e: Error) => notify(e.message, "error")).finally(() => {
        closingRef.current = false;
      });
      return;
    }
    void saveAndConfirmRow(sessionId, handle, resolvedRow).then((ok) => {
      if (ok) onScheduleDraft(resolvedRow.id);
    }).catch((e: Error) => notify(e.message, "error"));
  }, [activeBuffer.kind, closingRef, isNew, newPrompt, onCloseBuffer, onScheduleDraft, pendingRef, resolvedRow, sessionId]);

  const handleJumpFront = useCallback((): void => {
    if (isNew) {
      newPrompt.onJumpFront();
      return;
    }
    const handle = pendingRef.current;
    if (!handle || !handle.isSendable()) return;
    closingRef.current = true;
    void (async () => {
      if (activeBuffer.kind === "queued") {
        if (!resolvedRow || !(await saveAndConfirmRow(sessionId, handle, resolvedRow))) return;
        const order = [resolvedRow.id, ...queue.filter((m) => m.id !== resolvedRow.id).map((m) => m.id)];
        reorderQueue(sessionId, order);
        onCloseBuffer(closeTarget());
      } else if (resolvedRow) {
        if (!(await saveAndConfirmRow(sessionId, handle, resolvedRow))) return;
        await activateDraft(sessionId, resolvedRow.id, { front: true });
        onCloseBuffer();
      } else {
        await frontPrompt(sessionId, handle.getText(), [...handle.getAttachments()]);
        handle.discard();
        onCloseBuffer();
      }
    })().catch((e: Error) => notify(e.message, "error")).finally(() => {
      closingRef.current = false;
    });
  }, [activeBuffer.kind, closeTarget, closingRef, isNew, newPrompt, onCloseBuffer, pendingRef, queue, resolvedRow, sessionId]);

  const handleDemoteToDraft = useCallback((): void => {
    const handle = pendingRef.current;
    if (!handle || !resolvedRow || activeBuffer.kind !== "queued") return;
    closingRef.current = true;
    void saveAndConfirmRow(sessionId, handle, resolvedRow).then(async (ok) => {
      if (!ok) return;
      await queuedToDraft(sessionId, resolvedRow.id);
      onCloseBuffer();
    }).catch((e: Error) => notify(e.message, "error")).finally(() => {
      closingRef.current = false;
    });
  }, [activeBuffer.kind, closingRef, onCloseBuffer, pendingRef, resolvedRow, sessionId]);

  const handlePromoteToDocument = useCallback((): void => {
    const handle = pendingRef.current;
    if (!handle || !resolvedRow || activeBuffer.kind !== "draft") return;
    closingRef.current = true;
    void saveAndConfirmRow(sessionId, handle, resolvedRow).then(async (ok) => {
      if (!ok) return;
      await moveSessionDraftToDocument(sessionId, {
        ...resolvedRow,
        text: handle.getText(),
        attachments: [...handle.getAttachments()],
      });
      onCloseBuffer();
    }).catch((e: Error) => notify(e.message, "error")).finally(() => {
      closingRef.current = false;
    });
  }, [activeBuffer.kind, closingRef, onCloseBuffer, pendingRef, resolvedRow, sessionId]);

  const handleMoveToSession = useCallback((): void => {
    if (!onMoveDraft || !resolvedRow) return;
    const handle = pendingRef.current;
    if (!handle) return;
    void saveAndConfirmRow(sessionId, handle, resolvedRow).then((ok) => {
      if (ok) onMoveDraft(resolvedRow.id);
    }).catch((e: Error) => notify(e.message, "error"));
  }, [onMoveDraft, pendingRef, resolvedRow, sessionId]);

  return {
    handleSchedule,
    handleJumpFront,
    handleDemoteToDraft,
    handlePromoteToDocument,
    handleMoveToSession,
  };
}

function useBufferCloseActions(
  args: DesktopBufferActionsArgs,
  closeTarget: () => { kind: "queued" | "draft"; id: string } | undefined,
) {
  const { sessionId, activeBuffer, resolvedRow, pendingRef, bufferMeta, onCloseBuffer, closingRef } = args;
  const isNew = activeBuffer.kind === "new";

  const handleEscape = useCallback((): void => {
    if (isNew) return;
    const handle = pendingRef.current;
    if (!handle) {
      onCloseBuffer(closeTarget());
      return;
    }
    // Never discard an edit whose attachment is still encoding or hydrating.
    if (handle.isEncoding()) {
      notify("Waiting for attachment to finish encoding…", "info");
      return;
    }
    if (activeBuffer.kind === "queued" || bufferMeta.scheduled) {
      if (!handle.isDirty()) {
        handle.discard();
        onCloseBuffer(closeTarget());
      } else {
        handle.flashDirtyHint();
      }
      return;
    }
    closingRef.current = true;
    const finish = (): void => {
      closingRef.current = false;
      onCloseBuffer(closeTarget());
    };
    if (handle.isDirty() && handle.isSendable()) {
      void handle.save({ silent: true }).then((ok) => {
        if (ok) finish();
        else closingRef.current = false;
      }).catch((e: Error) => {
        closingRef.current = false;
        notify(e.message, "error");
      });
    } else if (!handle.isDirty()) {
      handle.discard();
      finish();
    } else {
      closingRef.current = false;
      handle.flashDirtyHint();
    }
  }, [activeBuffer.kind, bufferMeta.scheduled, closeTarget, closingRef, isNew, onCloseBuffer, pendingRef]);

  const handleDiscard = useCallback((): void => {
    pendingRef.current?.discard();
    onCloseBuffer(closeTarget());
  }, [closeTarget, onCloseBuffer, pendingRef]);

  const handleSaveAndClose = useCallback((): void => {
    const handle = pendingRef.current;
    if (!handle) return;
    closingRef.current = true;
    void saveAndConfirmRow(sessionId, handle, resolvedRow).then((ok) => {
      if (ok) onCloseBuffer(closeTarget());
    }).catch((e: Error) => notify(e.message, "error")).finally(() => {
      closingRef.current = false;
    });
  }, [closeTarget, closingRef, onCloseBuffer, pendingRef, resolvedRow, sessionId]);

  return { handleEscape, handleDiscard, handleSaveAndClose };
}

async function runDeliverBuffer({
  sessionId,
  kind,
  queue,
  row,
  handle,
  mode,
  target,
  nextCheckpoint,
}: {
  sessionId: string;
  kind: DesktopPromptBuffer["kind"];
  queue: readonly QueuedMessage[];
  row: QueuedMessage | null;
  handle: DesktopPendingBufferHandle;
  mode: Delivery | "steer" | "pause";
  target?: DeliveryTarget | undefined;
  nextCheckpoint: boolean;
}): Promise<void> {
  if (kind !== "new-draft" && row && !(await saveAndConfirmRow(sessionId, handle, row))) return;
  const text = handle.getText();
  const atts = [...handle.getAttachments()];
  if (mode === "steer") {
    // Server atomically removes the source draft/queued row (cancelling any
    // schedule timer) once steer is admitted under this single operation id.
    const consume = row
      ? { kind: kind === "queued" ? ("queued" as const) : ("draft" as const), id: row.id }
      : undefined;
    await currentTurnPrompt(sessionId, text, atts, target, nextCheckpoint, consume);
  } else if (mode === "front") {
    if (kind === "queued" && row) {
      // Reorder-only: respects Pause and never registers explicit_wait.
      const order = [row.id, ...queue.filter((m) => m.id !== row.id).map((m) => m.id)];
      reorderQueue(sessionId, order);
    } else if (row) {
      // Atomic server transition: removes draft, cancels timer, front-queues.
      await activateDraft(sessionId, row.id, { front: true });
    } else {
      await frontPrompt(sessionId, text, atts);
    }
  } else if (mode === "pause") {
    await pauseQueueConfirmed(sessionId);
    if (row) await activateDraft(sessionId, row.id);
    else await submitPrompt(sessionId, text, atts);
  } else {
    if (row) await activateDraft(sessionId, row.id);
    else await submitPrompt(sessionId, text, atts);
  }
  handle.discard();
}

function useHandleBufferSubmit(
  args: DesktopBufferActionsArgs,
  busy: boolean,
  closeTarget: () => { kind: "queued" | "draft"; id: string } | undefined,
  setDeliveryAnchor: (el: HTMLElement | null) => void,
): () => void {
  const {
    sessionId, queue, activeBuffer,
    resolvedRow, pendingRef, sendButtonRef, bufferMeta,
    newPrompt, onCloseBuffer, closingRef,
  } = args;
  const isNew = activeBuffer.kind === "new";
  return useCallback((): void => {
    if (isNew) {
      newPrompt.onSubmit();
      return;
    }
    const handle = pendingRef.current;
    if (!handle || !handle.isSendable()) return;
    if (activeBuffer.kind === "queued") {
      closingRef.current = true;
      void saveAndConfirmRow(sessionId, handle, resolvedRow).then((ok) => {
        if (ok) onCloseBuffer(closeTarget());
      }).catch((e: Error) => notify(e.message, "error")).finally(() => {
        closingRef.current = false;
      });
      return;
    }
    if (bufferMeta.scheduled || busy || queue.length > 0) {
      const prep = activeBuffer.kind === "new-draft"
        ? Promise.resolve(true)
        : saveAndConfirmRow(sessionId, handle, resolvedRow);
      void prep.then((ok) => {
        if (ok) setDeliveryAnchor(sendButtonRef.current);
      }).catch((e: Error) => notify(e.message, "error"));
      return;
    }
    closingRef.current = true;
    void (async () => {
      if (activeBuffer.kind === "new-draft" || !resolvedRow) {
        await submitPrompt(sessionId, handle.getText(), [...handle.getAttachments()]);
        handle.discard();
        onCloseBuffer();
        return;
      }
      if (!(await saveAndConfirmRow(sessionId, handle, resolvedRow))) return;
      await activateDraft(sessionId, resolvedRow.id);
      onCloseBuffer();
    })().catch((e: Error) => notify(e.message, "error")).finally(() => {
      closingRef.current = false;
    });
  }, [
    activeBuffer.kind, bufferMeta.scheduled, busy, closeTarget, closingRef,
    isNew, newPrompt, onCloseBuffer, pendingRef, queue.length,
    resolvedRow, sendButtonRef, sessionId, setDeliveryAnchor,
  ]);
}

export function useDesktopBufferActions(args: DesktopBufferActionsArgs) {
  const {
    sessionId, status, queue, activeBuffer, resolvedRow, pendingRef,
    sendButtonRef, newPrompt, onCloseBuffer, onOpenStop, closingRef,
  } = args;

  const [deliveryAnchor, setDeliveryAnchor] = useState<HTMLElement | null>(null);
  const isNew = activeBuffer.kind === "new";
  const busy = status === "busy";

  const closeTarget = useCallback((): { kind: "queued" | "draft"; id: string } | undefined => {
    if (
      activeBuffer.kind !== "new" &&
      activeBuffer.origin === "list" &&
      resolvedRow &&
      (activeBuffer.kind === "queued" || activeBuffer.kind === "draft")
    ) {
      return { kind: activeBuffer.kind, id: resolvedRow.id };
    }
    return undefined;
  }, [activeBuffer, resolvedRow]);

  const transfer = useBufferTransferActions(args, closeTarget);
  const closing = useBufferCloseActions(args, closeTarget);
  const handleSubmit = useHandleBufferSubmit(args, busy, closeTarget, setDeliveryAnchor);

  const handleSave = useCallback((): void => {
    if (isNew) {
      newPrompt.onSaveDraft();
      return;
    }
    void pendingRef.current?.save().catch((e: Error) => notify(e.message, "error"));
  }, [isNew, newPrompt, pendingRef]);

  const handleForce = useCallback((): void => {
    if (isNew) {
      newPrompt.onForce();
      return;
    }
    const handle = pendingRef.current;
    if (!handle || !handle.isSendable()) return;
    const prep = activeBuffer.kind === "new-draft"
      ? Promise.resolve(true)
      : saveAndConfirmRow(sessionId, handle, resolvedRow);
    void prep.then((ok) => {
      if (ok) setDeliveryAnchor(sendButtonRef.current);
    }).catch((e: Error) => notify(e.message, "error"));
  }, [activeBuffer.kind, isNew, newPrompt, pendingRef, resolvedRow, sendButtonRef, sessionId]);

  const deliverBuffer = useCallback(
    (mode: Delivery | "steer" | "pause", target?: DeliveryTarget, nextCheckpoint = false): void => {
      setDeliveryAnchor(null);
      const handle = pendingRef.current;
      if (!handle) return;
      closingRef.current = true;
      void runDeliverBuffer({
        sessionId, kind: activeBuffer.kind, queue, row: resolvedRow, handle,
        mode, target, nextCheckpoint,
      }).then(() => onCloseBuffer())
        .catch((e: Error) => notify(e.message, "error"))
        .finally(() => {
          closingRef.current = false;
        });
    },
    [activeBuffer.kind, closingRef, onCloseBuffer, pendingRef, queue, resolvedRow, sessionId],
  );

  return {
    deliveryAnchor,
    closeDelivery: () => setDeliveryAnchor(null),
    deliverBuffer,
    onStop: () => {
      // Confirmation dialog owns cancellation; never cancel before the user confirms.
      setDeliveryAnchor(null);
      onOpenStop();
    },
    handleSubmit,
    handleSave,
    handleForce,
    ...transfer,
    ...closing,
    toolbarPresentation: buildToolbarPresentation(args, busy),
  };
}
