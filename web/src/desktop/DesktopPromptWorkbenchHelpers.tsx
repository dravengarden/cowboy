import {
  type ReactNode,
  type RefObject,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Box, Button, DialogContentText, Paper } from "@mui/material";
import { Check, Send } from "@mui/icons-material";
import type { ComposerEditorHandle } from "../ComposerEditor";
import { DeliveryPicker } from "../DeliveryPicker";
import { Kbd, useConfirmEnter } from "../Kbd";
import { DesktopRegionShortcut, pendingContentCleared, PendingPanel } from "../PendingPanel";
import { ENTER_LABEL, MOD_LABEL } from "../platform";
import type { AvailableCommand, Status } from "../protocol";
import { fireLabel } from "../scheduleTime";
import type { SessionContext } from "../sessionContext";
import { ConfirmSheet } from "../Sheet";
import { confirmedPending, notify, type QueuedMessage, waitForPendingEdit } from "../store";
import { useRemoteQueueEditing } from "../queueEditingHold";
import { recoverPendingEditId } from "../pendingEditStore";
import { setActiveVimOwner } from "../vimModeStore";
import { DesktopComposerToolbar } from "./DesktopComposerToolbar";
import {
  DesktopPendingBufferEditor,
  type DesktopPendingBufferHandle,
} from "./DesktopPendingBufferEditor";
import {
  clearPromptChord,
  type createPromptChordTracker,
  evaluatePromptCanvasKey,
} from "./DesktopPromptVimShortcuts";
import type { useDesktopWorkspace } from "./DesktopWorkspaceController";
import {
  DESKTOP_FOCUS_NEW_PROMPT_EVENT,
  requestDesktopEditorInsert,
} from "./editorInsert";
import {
  adjacentSendOrderBuffer,
  consumePendingOpenSessionDraft,
  DESKTOP_NEW_SESSION_DRAFT_EVENT,
  DESKTOP_OPEN_SESSION_DRAFT_EVENT,
  DESKTOP_TOGGLE_EMPTY_DRAFTS_EVENT,
  type DesktopPromptBuffer,
  getLastSubmittedPending,
  newDraftBufferCmid,
  resetDesktopPromptStatus,
  resolveBufferRow,
  resolveLastSubmittedRow,
  setDesktopPromptStatus,
} from "./promptBuffer";
import type { NewPromptCallbacks, useDesktopBufferActions } from "./useDesktopBufferActions";
import { DESKTOP_FOCUS_PROMPT_SHORTCUT } from "./commands/workspaceShortcuts";

export function DesktopPendingPanels({
  sessionId,
  status,
  queue,
  drafts,
  emptyDraftOpen,
  nextAgentContext,
  commands,
  activeBuffer,
  resolvedRowId,
  preferredQueueFocus,
  onMoveDraft,
  onScheduleDraft,
  onEditRow,
  onNewDraft,
}: {
  sessionId: string;
  status: Status;
  queue: readonly QueuedMessage[];
  drafts: readonly QueuedMessage[];
  emptyDraftOpen: boolean;
  nextAgentContext?: SessionContext | undefined;
  commands: () => AvailableCommand[];
  activeBuffer: DesktopPromptBuffer;
  resolvedRowId: string | null;
  preferredQueueFocus: string | null;
  onMoveDraft?: ((id: string) => void) | undefined;
  onScheduleDraft: (id: string) => void;
  onEditRow: (kind: "queued" | "draft", msg: QueuedMessage, entryMode: "insert" | "normal") => void;
  onNewDraft: () => void;
}): React.JSX.Element | null {
  const remoteEditing = useRemoteQueueEditing(sessionId);
  if (queue.length === 0 && drafts.length === 0 && !emptyDraftOpen) return null;
  const queueAutoDrainHeld = activeBuffer.kind === "queued" ||
    Boolean(remoteEditing && queue.some((m) => m.id === remoteEditing));
  return (
    <Box data-composer-stack-slot="pending" sx={{ flexShrink: 0, minHeight: 0 }}>
      {queue.length > 0 && (
        <PendingPanel
          desktop
          keyboardOpen={false}
          kind="queued"
          sessionId={sessionId}
          items={[...queue]}
          status={status}
          nextAgentContext={nextAgentContext}
          commands={commands}
          desktopActiveEditId={activeBuffer.kind === "queued" ? resolvedRowId : null}
          desktopEditingHeld={queueAutoDrainHeld}
          desktopPreferredFocusId={preferredQueueFocus}
          onDesktopEditRow={(msg, entryMode) => onEditRow("queued", msg, entryMode)}
        />
      )}
      {(drafts.length > 0 || emptyDraftOpen) && (
        <PendingPanel
          desktop
          keyboardOpen={false}
          kind="draft"
          sessionId={sessionId}
          items={[...drafts]}
          status={status}
          commands={commands}
          onMoveDraft={onMoveDraft}
          onScheduleDraft={onScheduleDraft}
          desktopActiveEditId={activeBuffer.kind === "draft" ? resolvedRowId : null}
          desktopEmptyOpen={emptyDraftOpen && drafts.length === 0}
          onDesktopEditRow={(msg, entryMode) => onEditRow("draft", msg, entryMode)}
          onDesktopNewDraft={onNewDraft}
        />
      )}
    </Box>
  );
}

export function useWorkbenchEvents({
  sessionId,
  queue,
  drafts,
  activeBuffer,
  composerPaperRef,
  chordTrackerRef,
  setActiveBuffer,
  setEmptyDraftOpen,
  openBuffer,
  workspace,
}: {
  sessionId: string;
  queue: readonly QueuedMessage[];
  drafts: readonly QueuedMessage[];
  activeBuffer: DesktopPromptBuffer;
  composerPaperRef: RefObject<HTMLDivElement | null>;
  chordTrackerRef: RefObject<ReturnType<typeof createPromptChordTracker>>;
  setActiveBuffer: React.Dispatch<React.SetStateAction<DesktopPromptBuffer>>;
  setEmptyDraftOpen: React.Dispatch<React.SetStateAction<boolean>>;
  openBuffer: (next: DesktopPromptBuffer, opts?: { insertAfter?: boolean }) => boolean;
  workspace: ReturnType<typeof useDesktopWorkspace>;
}): void {
  const recoveredSessionRef = useRef<string | null>(null);
  const userInteractedRef = useRef(false);
  useEffect(() => {
    userInteractedRef.current = false;
    recoveredSessionRef.current = null;
  }, [sessionId]);
  useEffect(() => {
    if (activeBuffer.kind !== "new") {
      userInteractedRef.current = true;
      recoveredSessionRef.current = sessionId;
    }
  }, [activeBuffer.kind, sessionId]);
  useEffect(() => {
    if (
      recoveredSessionRef.current === sessionId ||
      userInteractedRef.current ||
      activeBuffer.kind !== "new"
    ) return;
    const hydrated = confirmedPending(sessionId) !== undefined ||
      queue.length > 0 ||
      drafts.length > 0;
    if (!hydrated) return;
    recoveredSessionRef.current = sessionId;
    const qId = recoverPendingEditId(sessionId, "queued", queue);
    if (qId) {
      const row = queue.find((m) => m.id === qId);
      setActiveBuffer({
        kind: "queued",
        id: qId,
        ...(row?.cmid ? { cmid: row.cmid } : {}),
        origin: "new",
        entryMode: "normal",
      });
      return;
    }
    const dId = recoverPendingEditId(sessionId, "draft", drafts);
    if (dId) {
      const row = drafts.find((m) => m.id === dId);
      setActiveBuffer({
        kind: "draft",
        id: dId,
        ...(row?.cmid ? { cmid: row.cmid } : {}),
        origin: "new",
        entryMode: "normal",
      });
    }
  }, [activeBuffer.kind, drafts, queue, sessionId, setActiveBuffer]);

  useEffect(() => {
    const queuedOpen = consumePendingOpenSessionDraft(sessionId);
    if (queuedOpen) {
      openBuffer({
        kind: "draft",
        id: queuedOpen.id ?? "",
        ...(queuedOpen.cmid ? { cmid: queuedOpen.cmid } : {}),
        origin: "new",
        entryMode: "normal",
      });
    }
  }, [openBuffer, sessionId]);

  useEffect(() => {
    const el = composerPaperRef.current;
    const onFocusNew = (event: Event): void => {
      if (activeBuffer.kind === "new") return;
      const switchedSync = openBuffer({ kind: "new" }, { insertAfter: true });
      if (!switchedSync) event.preventDefault();
    };
    const onToggleEmptyDrafts = (): void => {
      if (drafts.length > 0) return;
      setEmptyDraftOpen((open) => {
        const next = !open;
        requestAnimationFrame(() =>
          workspace.focusRegion(next ? "prompt.draft" : "prompt.composer")
        );
        return next;
      });
    };
    const onNewSessionDraft = (): void => {
      setEmptyDraftOpen(false);
      openBuffer({
        kind: "new-draft",
        cmid: newDraftBufferCmid(),
        origin: "list",
        entryMode: "insert",
      });
    };
    const onOpenSessionDraft = (e: Event): void => {
      const detail = (e as CustomEvent<{ sessionId?: string; id?: string; cmid?: string }>).detail;
      if (!detail || detail.sessionId !== sessionId) return;
      openBuffer({
        kind: "draft",
        id: detail.id ?? "",
        ...(detail.cmid ? { cmid: detail.cmid } : {}),
        origin: "new",
        entryMode: "normal",
      });
    };
    el?.addEventListener(DESKTOP_FOCUS_NEW_PROMPT_EVENT, onFocusNew);
    globalThis.addEventListener(DESKTOP_TOGGLE_EMPTY_DRAFTS_EVENT, onToggleEmptyDrafts);
    globalThis.addEventListener(DESKTOP_NEW_SESSION_DRAFT_EVENT, onNewSessionDraft);
    globalThis.addEventListener(DESKTOP_OPEN_SESSION_DRAFT_EVENT, onOpenSessionDraft);
    return () => {
      el?.removeEventListener(DESKTOP_FOCUS_NEW_PROMPT_EVENT, onFocusNew);
      globalThis.removeEventListener(DESKTOP_TOGGLE_EMPTY_DRAFTS_EVENT, onToggleEmptyDrafts);
      globalThis.removeEventListener(DESKTOP_NEW_SESSION_DRAFT_EVENT, onNewSessionDraft);
      globalThis.removeEventListener(DESKTOP_OPEN_SESSION_DRAFT_EVENT, onOpenSessionDraft);
      clearPromptChord(chordTrackerRef.current);
    };
  }, [
    activeBuffer.kind,
    chordTrackerRef,
    composerPaperRef,
    drafts.length,
    openBuffer,
    sessionId,
    setActiveBuffer,
    setEmptyDraftOpen,
    workspace,
  ]);
}

export function useWorkbenchStatusSync({
  activeBuffer,
  resolved,
  bufferMeta,
  holdStatus,
  toolbarPresentation,
}: {
  activeBuffer: DesktopPromptBuffer;
  resolved: { row: QueuedMessage; index: number; total: number } | null;
  bufferMeta: { dirty: boolean; scheduled: boolean };
  holdStatus: import("../queueEditingHold").QueueEditHoldStatus;
  toolbarPresentation: { sendLabel: string; canForce: boolean };
}): void {
  useEffect(() => {
    const kind = activeBuffer.kind === "new"
      ? "new"
      : activeBuffer.kind === "queued"
      ? "queued"
      : activeBuffer.kind === "new-draft"
      ? "new-draft"
      : bufferMeta.scheduled
      ? "scheduled"
      : "draft";
    const scheduleLabel = resolved?.row.schedule
      ? fireLabel(resolved.row.schedule.fire_at_ms)
      : undefined;
    setDesktopPromptStatus({
      kind,
      ...(resolved ? { index: resolved.index, total: resolved.total } : {}),
      dirty: activeBuffer.kind === "new" ? false : bufferMeta.dirty,
      holdStatus: activeBuffer.kind === "queued" ? holdStatus : "none",
      ...(scheduleLabel ? { scheduleLabel } : {}),
      sendLabel: toolbarPresentation.sendLabel,
      canForce: toolbarPresentation.canForce,
    });
    return () => resetDesktopPromptStatus();
  }, [
    activeBuffer.kind,
    bufferMeta.dirty,
    bufferMeta.scheduled,
    holdStatus,
    resolved,
    toolbarPresentation.canForce,
    toolbarPresentation.sendLabel,
  ]);
}

export function useWorkbenchCanvasKeys({
  sessionId,
  queue,
  drafts,
  activeBuffer,
  newPrompt,
  chordTrackerRef,
  openBuffer,
  onSaveAndClose,
  onDiscard,
}: {
  sessionId: string;
  queue: readonly QueuedMessage[];
  drafts: readonly QueuedMessage[];
  activeBuffer: DesktopPromptBuffer;
  newPrompt: NewPromptCallbacks;
  chordTrackerRef: RefObject<ReturnType<typeof createPromptChordTracker>>;
  openBuffer: (next: DesktopPromptBuffer) => void;
  onSaveAndClose: () => void;
  onDiscard: () => void;
}): (event: React.KeyboardEvent) => void {
  return useCallback((event: React.KeyboardEvent): void => {
    const isNew = activeBuffer.kind === "new";
    const isEmpty = isNew && newPrompt.getText().trim() === "" && !newPrompt.sendable;
    const lastMatch = isNew
      ? resolveLastSubmittedRow(getLastSubmittedPending(sessionId), queue, drafts)
      : null;
    const result = evaluatePromptCanvasKey({
      event: event.nativeEvent,
      tracker: chordTrackerRef.current,
      isNewPrompt: isNew,
      isEmpty,
      hasCompletionOpen: document.querySelector(".cm-tooltip-autocomplete") !== null,
      canRecallLast: lastMatch !== null,
      canCycleQueue: queue.some((m) => !m.currentTurn && m.status === undefined),
    });
    if (!result.consumed) return;
    event.preventDefault();
    event.stopPropagation();
    event.nativeEvent.stopImmediatePropagation();
    if (!result.action) return;
    if (result.action.kind === "recall-last-pending" && lastMatch) {
      openBuffer({
        kind: lastMatch.kind,
        id: lastMatch.row.id,
        ...(lastMatch.row.cmid ? { cmid: lastMatch.row.cmid } : {}),
        origin: "new",
        entryMode: "insert",
      });
    } else if (
      result.action.kind === "cycle-send-order-prev" ||
      result.action.kind === "cycle-send-order-next"
    ) {
      const target = adjacentSendOrderBuffer(
        activeBuffer,
        queue,
        result.action.kind === "cycle-send-order-prev" ? -1 : 1,
      );
      if (target) openBuffer(target);
    } else if (result.action.kind === "save-and-close") onSaveAndClose();
    else if (result.action.kind === "discard-and-close") onDiscard();
  }, [
    activeBuffer,
    chordTrackerRef,
    drafts,
    newPrompt,
    onDiscard,
    onSaveAndClose,
    openBuffer,
    queue,
    sessionId,
  ]);
}

export function useWorkbenchBufferState({
  sessionId,
  queue,
  drafts,
  pendingRef,
  workspace,
}: {
  sessionId: string;
  queue: readonly QueuedMessage[];
  drafts: readonly QueuedMessage[];
  pendingRef: RefObject<DesktopPendingBufferHandle | null>;
  workspace: ReturnType<typeof useDesktopWorkspace>;
}) {
  const [activeBuffer, setActiveBuffer] = useState<DesktopPromptBuffer>({ kind: "new" });
  const [emptyDraftOpen, setEmptyDraftOpen] = useState(false);
  const [pendingSwitch, setPendingSwitch] = useState<{
    buffer: DesktopPromptBuffer;
    insertAfter: boolean;
  } | null>(null);
  const [bufferMeta, setBufferMeta] = useState({
    sendable: false,
    dirty: false,
    saving: false,
    scheduled: false,
    holdStatus: "none" as import("../queueEditingHold").QueueEditHoldStatus,
  });

  const resolved = useMemo(
    () => resolveBufferRow(activeBuffer, queue, drafts),
    [activeBuffer, drafts, queue],
  );
  useEffect(() => {
    setActiveBuffer({ kind: "new" });
    setEmptyDraftOpen(false);
    setPendingSwitch(null);
  }, [sessionId]);
  useEffect(() => {
    if (activeBuffer.kind === "new") setActiveVimOwner("default");
  }, [activeBuffer.kind, sessionId]);
  useEffect(() => {
    if (
      resolved &&
      (activeBuffer.kind === "queued" || activeBuffer.kind === "draft") &&
      resolved.row.id !== activeBuffer.id
    ) setActiveBuffer({ ...activeBuffer, id: resolved.row.id });
  }, [activeBuffer, resolved]);
  useEffect(() => {
    if (drafts.length > 0 && emptyDraftOpen) setEmptyDraftOpen(false);
  }, [drafts.length, emptyDraftOpen]);

  const applySwitch = useCallback((next: DesktopPromptBuffer, insertAfter = false): void => {
    setActiveBuffer(next);
    requestAnimationFrame(() => {
      workspace.focusRegion("prompt.composer");
      if (insertAfter) requestAnimationFrame(requestDesktopEditorInsert);
    });
  }, [workspace]);

  const closeBuffer = useCallback(
    (focusTarget?: { kind: "queued" | "draft"; id: string }): void => {
      setActiveBuffer({ kind: "new" });
      if (focusTarget) {
        requestAnimationFrame(() => {
          const rowEl = document.querySelector<HTMLElement>(
            `[data-desktop-region="prompt.${focusTarget.kind}"] [data-desktop-item="${CSS.escape(focusTarget.id)}"]`,
          );
          if (rowEl) {
            workspace.focusRegion(`prompt.${focusTarget.kind}`);
            requestAnimationFrame(() => rowEl.focus({ preventScroll: true }));
          } else workspace.focusRegion("prompt.composer");
        });
      } else requestAnimationFrame(() => workspace.focusRegion("prompt.composer"));
    },
    [workspace],
  );

  const openBuffer = useCallback(
    (next: DesktopPromptBuffer, opts?: { insertAfter?: boolean }): boolean => {
      const insertAfter = opts?.insertAfter === true;
      const handle = pendingRef.current;
      if (activeBuffer.kind !== "new" && handle?.isEncoding()) {
        notify("Waiting for attachment to finish encoding…", "info");
        return false;
      }
      const transactionalDirty =
        (activeBuffer.kind === "queued" || (activeBuffer.kind === "draft" && bufferMeta.scheduled)) &&
        handle?.isDirty();
      if (transactionalDirty) {
        setPendingSwitch({ buffer: next, insertAfter });
        return false;
      }
      const needsDraftFlush = handle !== null && (
        (activeBuffer.kind === "new-draft" &&
          !pendingContentCleared(handle.getText(), [...handle.getAttachments()])) ||
        (activeBuffer.kind === "draft" && !bufferMeta.scheduled && handle.isDirty())
      );
      if (needsDraftFlush) {
        void handle.save({ silent: true }).then((ok) => {
          if (ok) applySwitch(next, insertAfter);
        }).catch((e: Error) => notify(e.message, "error"));
        return false;
      }
      applySwitch(next, insertAfter);
      return true;
    },
    [activeBuffer.kind, applySwitch, bufferMeta.scheduled, pendingRef],
  );

  const confirmSwitch = useCallback((resolution: "save" | "discard"): void => {
    const pending = pendingSwitch;
    if (!pending) return;
    setPendingSwitch(null);
    if (resolution === "save") {
      const rowId = resolved?.row.id;
      void (async () => {
        const ok = await pendingRef.current?.save({ silent: true });
        if (!ok) return;
        // Wait for authoritative server acknowledgement before unmounting and
        // releasing a queued or scheduled-draft edit hold.
        if (rowId && !rowId.startsWith("opt-")) {
          await waitForPendingEdit(sessionId, rowId);
        }
        applySwitch(pending.buffer, pending.insertAfter);
      })().catch((e: Error) => notify(e.message, "error"));
    } else {
      pendingRef.current?.discard();
      applySwitch(pending.buffer, pending.insertAfter);
    }
  }, [applySwitch, pendingRef, pendingSwitch, resolved?.row.id, sessionId]);
  useConfirmEnter(pendingSwitch !== null, () => confirmSwitch("save"));

  return {
    activeBuffer,
    setActiveBuffer,
    emptyDraftOpen,
    setEmptyDraftOpen,
    pendingSwitch,
    setPendingSwitch,
    bufferMeta,
    setBufferMeta,
    resolved,
    closeBuffer,
    openBuffer,
    confirmSwitch,
  };
}

export interface DesktopWorkbenchCanvasProps {
  composerPaperRef: RefObject<HTMLDivElement | null>;
  pendingRef: RefObject<DesktopPendingBufferHandle | null>;
  activeEditorProxy: RefObject<ComposerEditorHandle | null>;
  sendButtonRef: RefObject<HTMLButtonElement | null>;
  sessionId: string;
  status: Status;
  handedOff: boolean;
  overlayOpen: boolean;
  currentTurnAvailable: boolean;
  commands: () => AvailableCommand[];
  activeBuffer: DesktopPromptBuffer;
  bufferKey: string;
  resolved: { row: QueuedMessage; index: number; total: number } | null;
  bufferMeta: { scheduled: boolean };
  setBufferMeta: (snapshot: {
    sendable: boolean;
    dirty: boolean;
    saving: boolean;
    scheduled: boolean;
    holdStatus: import("../queueEditingHold").QueueEditHoldStatus;
  }) => void;
  setActiveBuffer: React.Dispatch<React.SetStateAction<DesktopPromptBuffer>>;
  closeBuffer: () => void;
  newPrompt: NewPromptCallbacks;
  newPromptCanvas: ReactNode;
  actions: ReturnType<typeof useDesktopBufferActions>;
  onMoveDraft?: ((id: string) => void) | undefined;
  onCanvasKeyDownCapture: (event: React.KeyboardEvent) => void;
}

export function DesktopWorkbenchCanvas({
  composerPaperRef, pendingRef, activeEditorProxy, sendButtonRef, sessionId,
  status, handedOff, overlayOpen, currentTurnAvailable, commands, activeBuffer,
  bufferKey, resolved, bufferMeta, setBufferMeta, setActiveBuffer, closeBuffer,
  newPrompt, newPromptCanvas, actions, onMoveDraft, onCanvasKeyDownCapture,
}: DesktopWorkbenchCanvasProps): React.JSX.Element {
  const isNew = activeBuffer.kind === "new";
  return (
    <Paper
      ref={composerPaperRef}
      data-composer-stack-slot="primary"
      data-desktop-region="prompt.composer"
      data-desktop-focus-default={true}
      tabIndex={-1}
      variant="elevation"
      elevation={0}
      onKeyDownCapture={onCanvasKeyDownCapture}
      sx={{
        position: "relative",
        display: handedOff ? "none" : "flex",
        flexDirection: "column",
        flex: 1,
        minHeight: 0,
        border: 0,
        borderRadius: 0,
        boxShadow: "none",
        "&:focus-visible, &:focus-within": { outline: "none", boxShadow: "none" },
      }}
    >
      <Suspense fallback={null}>
        <DesktopRegionShortcut
          shortcut={DESKTOP_FOCUS_PROMPT_SHORTCUT}
          title="Focus prompt editor"
          showWhenPane="prompt"
          hideWhenRegion="prompt.composer"
          sx={{ position: "absolute", top: 10, right: 10, zIndex: 4 }}
        />
      </Suspense>
      <Box
        data-desktop-active-editor={isNew ? "true" : undefined}
        sx={{
          display: isNew ? "flex" : "none",
          flex: 1,
          minHeight: 0,
          flexDirection: "column",
          position: "relative",
        }}
      >
        {newPromptCanvas}
      </Box>
      {!isNew && (
        <DesktopPendingBufferEditor
          key={bufferKey}
          ref={pendingRef}
          sessionId={sessionId}
          kind={activeBuffer.kind}
          row={resolved?.row ?? null}
          draftCmid={activeBuffer.kind === "new-draft" || activeBuffer.kind === "draft"
            ? activeBuffer.cmid
            : undefined}
          index={resolved?.index}
          total={resolved?.total}
          entryMode={activeBuffer.entryMode}
          commands={commands}
          onStateChange={setBufferMeta}
          onMaterializedDraft={(id, cmid) => {
            setActiveBuffer({
              kind: "draft",
              id,
              ...(cmid ? { cmid } : {}),
              origin: activeBuffer.origin,
              entryMode: "insert",
            });
          }}
          onSourceVanished={(recovered) => {
            if (recovered) {
              setActiveBuffer({
                kind: "draft",
                id: recovered.id ?? "",
                cmid: recovered.cmid,
                origin: activeBuffer.origin,
                entryMode: "normal",
              });
            } else closeBuffer();
          }}
          onEscape={actions.handleEscape}
          onBack={actions.handleEscape}
          onDiscard={actions.handleDiscard}
          onDemoteToDraft={activeBuffer.kind === "queued" ? actions.handleDemoteToDraft : undefined}
          onMoveToSession={activeBuffer.kind === "draft" && onMoveDraft ? actions.handleMoveToSession : undefined}
          onPromoteToDocument={activeBuffer.kind === "draft" && !bufferMeta.scheduled ? actions.handlePromoteToDocument : undefined}
        />
      )}
      <DesktopComposerToolbar
        sessionId={sessionId}
        status={status}
        editorRef={activeEditorProxy}
        sendButtonRef={sendButtonRef}
        canInsert={newPrompt.canInsert}
        sendable={actions.toolbarPresentation.sendable}
        canSave={actions.toolbarPresentation.canSave}
        canSchedule={actions.toolbarPresentation.canSchedule}
        canJumpFront={actions.toolbarPresentation.canJumpFront}
        canForce={actions.toolbarPresentation.canForce}
        canCurrentTurn={isNew ? newPrompt.canCurrentTurn : currentTurnAvailable}
        onCurrentTurn={isNew ? newPrompt.onCurrentTurn : actions.handleForce}
        pending={actions.toolbarPresentation.pending}
        progress={actions.toolbarPresentation.progress}
        sendLabel={actions.toolbarPresentation.sendLabel}
        sendIcon={actions.toolbarPresentation.sendIconKind === "check"
          ? <Check sx={{ fontSize: "1rem" }} />
          : actions.toolbarPresentation.sendIconKind === "send"
          ? <Send sx={{ fontSize: "1rem" }} />
          : undefined}
        sendDescription={actions.toolbarPresentation.sendDescription}
        saveLabel={actions.toolbarPresentation.saveLabel}
        saveTitle={actions.toolbarPresentation.saveTitle}
        scheduleLabel={actions.toolbarPresentation.scheduleLabel}
        scheduleTitle={actions.toolbarPresentation.scheduleTitle}
        unavailableReason={newPrompt.unavailableReason}
        overlayOpen={overlayOpen || actions.deliveryAnchor !== null}
        onAttach={() => {
          if (isNew) newPrompt.onAttach();
          else pendingRef.current?.attach();
        }}
        onSaveDraft={actions.handleSave}
        onSchedule={actions.handleSchedule}
        onJumpFront={actions.handleJumpFront}
        onForce={actions.handleForce}
        onSubmit={actions.handleSubmit}
      />
    </Paper>
  );
}

export function DesktopWorkbenchDialogs({
  sessionId,
  status,
  paused,
  currentTurnAvailable,
  queuedAhead,
  activeBuffer,
  deliveryAnchor,
  onCloseDelivery,
  onDeliverBuffer,
  onStop,
  pendingSwitch,
  onCloseSwitch,
  onConfirmSwitch,
}: {
  sessionId: string;
  status: Status;
  paused: boolean;
  currentTurnAvailable: boolean;
  queuedAhead: number;
  activeBuffer: DesktopPromptBuffer;
  deliveryAnchor: HTMLElement | null;
  onCloseDelivery: () => void;
  onDeliverBuffer: ReturnType<typeof useDesktopBufferActions>["deliverBuffer"];
  onStop: () => void;
  pendingSwitch: DesktopPromptBuffer | null;
  onCloseSwitch: () => void;
  onConfirmSwitch: (resolution: "save" | "discard") => void;
}): React.JSX.Element {
  return (
    <>
      {deliveryAnchor && (
        <DeliveryPicker
          sessionId={sessionId}
          anchor={deliveryAnchor}
          busy={status === "busy"}
          steerReady={currentTurnAvailable}
          paused={paused}
          queueCount={queuedAhead}
          queued={activeBuffer.kind === "queued"}
          onClose={onCloseDelivery}
          onNextTurn={() => onDeliverBuffer("back")}
          onQueueFront={() => onDeliverBuffer("front")}
          onSteer={(target, nextCheckpoint) =>
            onDeliverBuffer("steer", target, nextCheckpoint)}
          onSendAndPause={() => onDeliverBuffer("pause")}
          onStop={onStop}
        />
      )}
      <ConfirmSheet
        open={pendingSwitch !== null}
        onClose={onCloseSwitch}
        title="Unsaved changes in current message"
        actions={
          <>
            <Button color="inherit" onClick={onCloseSwitch}>
              Keep editing
              <Kbd keys="Esc" />
            </Button>
            <Button color="error" onClick={() => onConfirmSwitch("discard")}>
              Discard &amp; switch
            </Button>
            <Button variant="contained" onClick={() => onConfirmSwitch("save")}>
              Save &amp; switch
              <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
            </Button>
          </>
        }
      >
        <DialogContentText>
          Save or discard your edits before opening another message.
        </DialogContentText>
      </ConfirmSheet>
    </>
  );
}
