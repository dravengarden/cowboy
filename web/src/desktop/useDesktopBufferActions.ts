import { type MutableRefObject, type RefObject, useCallback, useState } from "react";
import type { DeliveryTarget } from "../currentTurnDelivery";
import { moveSessionDraftToDocument } from "../documents/sessionDraftImport";
import type { Delivery, Status } from "../protocol";
import {
  activateDraft,
  cancelTurn,
  currentTurnPrompt,
  frontPrompt,
  notify,
  pauseQueueConfirmed,
  type QueuedMessage,
  queuedToDraft,
  removeDraft,
  removeQueued,
  reorderQueue,
  requestSendQueued,
  submitPrompt,
} from "../store";
import type { DesktopPendingBufferHandle } from "./DesktopPendingBufferEditor";
import {
  type DesktopPromptBuffer,
  recordLastSubmittedPending,
} from "./promptBuffer";

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
      void handle.save({ silent: true }).then(async (ok) => {
        if (!ok) return;
        await queuedToDraft(sessionId, resolvedRow.id);
        onCloseBuffer();
        onScheduleDraft(resolvedRow.id);
      }).catch((e: Error) => notify(e.message, "error")).finally(() => {
        closingRef.current = false;
      });
      return;
    }
    void handle.save({ silent: true }).then((ok) => {
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
        if (!resolvedRow || !(await handle.save({ silent: true }))) return;
        const order = [resolvedRow.id, ...queue.filter((m) => m.id !== resolvedRow.id).map((m) => m.id)];
        reorderQueue(sessionId, order);
        onCloseBuffer(closeTarget());
      } else {
        await frontPrompt(sessionId, handle.getText(), [...handle.getAttachments()]);
        if (resolvedRow) await removeDraft(sessionId, resolvedRow.id);
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
    void handle.save({ silent: true }).then(async (ok) => {
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
    void handle.save({ silent: true }).then(async (ok) => {
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
    void pendingRef.current?.save({ silent: true }).then((ok) => {
      if (ok) onMoveDraft(resolvedRow.id);
    }).catch((e: Error) => notify(e.message, "error"));
  }, [onMoveDraft, pendingRef, resolvedRow]);

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
  const { activeBuffer, pendingRef, bufferMeta, onCloseBuffer, closingRef } = args;
  const isNew = activeBuffer.kind === "new";

  const handleEscape = useCallback((): void => {
    if (isNew) return;
    const handle = pendingRef.current;
    if (!handle) {
      onCloseBuffer(closeTarget());
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
      void handle.save({ silent: true }).catch(() => undefined).finally(finish);
    } else {
      handle.discard();
      finish();
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
    void handle.save({ silent: true }).then((ok) => {
      if (ok) onCloseBuffer(closeTarget());
    }).catch((e: Error) => notify(e.message, "error")).finally(() => {
      closingRef.current = false;
    });
  }, [closeTarget, closingRef, onCloseBuffer, pendingRef]);

  return { handleEscape, handleDiscard, handleSaveAndClose };
}

async function runDeliverBuffer({
  sessionId,
  kind,
  row,
  handle,
  mode,
  target,
  nextCheckpoint,
}: {
  sessionId: string;
  kind: DesktopPromptBuffer["kind"];
  row: QueuedMessage | null;
  handle: DesktopPendingBufferHandle;
  mode: Delivery | "steer" | "pause";
  target?: DeliveryTarget | undefined;
  nextCheckpoint: boolean;
}): Promise<void> {
  const text = handle.getText();
  const atts = [...handle.getAttachments()];
  if (mode === "steer") {
    await currentTurnPrompt(sessionId, text, atts, target, nextCheckpoint);
    if (row) {
      if (kind === "queued") await removeQueued(sessionId, row.id);
      else await removeDraft(sessionId, row.id);
    }
  } else if (mode === "front") {
    if (kind === "queued" && row) await requestSendQueued(sessionId, row.id);
    else {
      await frontPrompt(sessionId, text, atts);
      if (row) await removeDraft(sessionId, row.id);
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
    sessionId, paused, serverConnected, queue, activeBuffer,
    resolvedRow, pendingRef, sendButtonRef, bufferMeta,
    newPrompt, onCloseBuffer, closingRef,
  } = args;
  const isNew = activeBuffer.kind === "new";
  return useCallback((): void => {
    if (isNew) {
      if (busy || paused || queue.length > 0 || !serverConnected) {
        recordLastSubmittedPending(sessionId, {
          kind: "queued",
          textHint: newPrompt.getText(),
          at: Date.now(),
        });
      } else recordLastSubmittedPending(sessionId, null);
      newPrompt.onSubmit();
      return;
    }
    const handle = pendingRef.current;
    if (!handle || !handle.isSendable()) return;
    if (activeBuffer.kind === "queued") {
      closingRef.current = true;
      void handle.save({ silent: true }).then((ok) => {
        if (ok) onCloseBuffer(closeTarget());
      }).catch((e: Error) => notify(e.message, "error")).finally(() => {
        closingRef.current = false;
      });
      return;
    }
    if (bufferMeta.scheduled || busy || queue.length > 0) {
      const prep = activeBuffer.kind === "new-draft"
        ? Promise.resolve(true)
        : handle.save({ silent: true });
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
      if (!(await handle.save({ silent: true }))) return;
      await activateDraft(sessionId, resolvedRow.id);
      onCloseBuffer();
    })().catch((e: Error) => notify(e.message, "error")).finally(() => {
      closingRef.current = false;
    });
  }, [
    activeBuffer.kind, bufferMeta.scheduled, busy, closeTarget, closingRef,
    isNew, newPrompt, onCloseBuffer, paused, pendingRef, queue.length,
    resolvedRow, sendButtonRef, serverConnected, sessionId, setDeliveryAnchor,
  ]);
}

export function useDesktopBufferActions(args: DesktopBufferActionsArgs) {
  const {
    sessionId, status, activeBuffer, resolvedRow, pendingRef,
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
      recordLastSubmittedPending(sessionId, {
        kind: "draft",
        textHint: newPrompt.getText(),
        at: Date.now(),
      });
      newPrompt.onSaveDraft();
      return;
    }
    void pendingRef.current?.save().catch((e: Error) => notify(e.message, "error"));
  }, [isNew, newPrompt, pendingRef, sessionId]);

  const handleForce = useCallback((): void => {
    if (isNew) {
      newPrompt.onForce();
      return;
    }
    const handle = pendingRef.current;
    if (!handle || !handle.isSendable()) return;
    const prep = activeBuffer.kind === "new-draft"
      ? Promise.resolve(true)
      : handle.save({ silent: true });
    void prep.then((ok) => {
      if (ok) setDeliveryAnchor(sendButtonRef.current);
    }).catch((e: Error) => notify(e.message, "error"));
  }, [activeBuffer.kind, isNew, newPrompt, pendingRef, sendButtonRef]);

  const deliverBuffer = useCallback(
    (mode: Delivery | "steer" | "pause", target?: DeliveryTarget, nextCheckpoint = false): void => {
      setDeliveryAnchor(null);
      const handle = pendingRef.current;
      if (!handle) return;
      closingRef.current = true;
      void runDeliverBuffer({
        sessionId, kind: activeBuffer.kind, row: resolvedRow, handle,
        mode, target, nextCheckpoint,
      }).then(() => onCloseBuffer())
        .catch((e: Error) => notify(e.message, "error"))
        .finally(() => {
          closingRef.current = false;
        });
    },
    [activeBuffer.kind, closingRef, onCloseBuffer, pendingRef, resolvedRow, sessionId],
  );

  return {
    deliveryAnchor,
    closeDelivery: () => setDeliveryAnchor(null),
    deliverBuffer,
    onStop: () => {
      setDeliveryAnchor(null);
      cancelTurn(sessionId);
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
