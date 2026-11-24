import { type ReactNode, type RefObject, useMemo, useRef } from "react";
import type { ComposerEditorHandle } from "../ComposerEditor";
import type { AvailableCommand, Status } from "../protocol";
import type { SessionContext } from "../sessionContext";
import type { QueuedMessage } from "../store";
import { useQueueEditHoldStatus } from "../queueEditingHold";
import type { DesktopPendingBufferHandle } from "./DesktopPendingBufferEditor";
import { createPromptChordTracker } from "./DesktopPromptVimShortcuts";
import {
  DesktopPendingPanels,
  DesktopWorkbenchCanvas,
  DesktopWorkbenchDialogs,
  useWorkbenchBufferState,
  useWorkbenchCanvasKeys,
  useWorkbenchEvents,
  useWorkbenchStatusSync,
} from "./DesktopPromptWorkbenchHelpers";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { requestDesktopEditorInsert } from "./editorInsert";
import {
  getLastSubmittedPending,
  resolveLastSubmittedRow,
} from "./promptBuffer";
import {
  type NewPromptCallbacks,
  useDesktopBufferActions,
} from "./useDesktopBufferActions";

export interface DesktopPromptWorkbenchProps {
  sessionId: string;
  status: Status;
  paused: boolean;
  serverConnected: boolean;
  currentTurnAvailable: boolean;
  queue: readonly QueuedMessage[];
  drafts: readonly QueuedMessage[];
  nextAgentContext?: SessionContext | undefined;
  commands: () => AvailableCommand[];
  handedOff: boolean;
  overlayOpen: boolean;
  newPromptEditorRef: RefObject<ComposerEditorHandle | null>;
  sendButtonRef: RefObject<HTMLButtonElement | null>;
  newPrompt: NewPromptCallbacks;
  beforeCanvas: ReactNode;
  newPromptCanvas: ReactNode;
  onMoveDraft?: ((id: string) => void) | undefined;
  onScheduleDraft: (id: string) => void;
  onOpenStop: () => void;
}

export function DesktopPromptWorkbench(
  props: DesktopPromptWorkbenchProps,
): React.JSX.Element {
  const {
    sessionId, status, paused, serverConnected, currentTurnAvailable,
    queue, drafts, nextAgentContext, commands, handedOff, overlayOpen,
    newPromptEditorRef, sendButtonRef, newPrompt, beforeCanvas,
    newPromptCanvas, onMoveDraft, onScheduleDraft, onOpenStop,
  } = props;

  const workspace = useDesktopWorkspace();
  const holdStatus = useQueueEditHoldStatus(sessionId);
  const pendingRef = useRef<DesktopPendingBufferHandle | null>(null);
  const closingRef = useRef(false);
  const chordTrackerRef = useRef(createPromptChordTracker());
  const composerPaperRef = useRef<HTMLDivElement | null>(null);

  const state = useWorkbenchBufferState({
    sessionId, queue, drafts, pendingRef, workspace,
  });

  useWorkbenchEvents({
    sessionId, queue, drafts, activeBuffer: state.activeBuffer,
    bufferMeta: state.bufferMeta, pendingRef, composerPaperRef, chordTrackerRef,
    setActiveBuffer: state.setActiveBuffer, setEmptyDraftOpen: state.setEmptyDraftOpen,
    openBuffer: state.openBuffer, workspace,
  });

  const actions = useDesktopBufferActions({
    sessionId, status, paused, serverConnected, queue,
    activeBuffer: state.activeBuffer, resolvedRow: state.resolved?.row ?? null,
    pendingRef, sendButtonRef, bufferMeta: state.bufferMeta, newPrompt,
    onScheduleDraft, onMoveDraft, onCloseBuffer: state.closeBuffer,
    onOpenStop, closingRef,
  });

  useWorkbenchStatusSync({
    activeBuffer: state.activeBuffer, resolved: state.resolved,
    bufferMeta: state.bufferMeta, holdStatus,
    toolbarPresentation: actions.toolbarPresentation,
  });

  const activeEditorProxy = useMemo<RefObject<ComposerEditorHandle | null>>(() => ({
    get current(): ComposerEditorHandle | null {
      return state.activeBuffer.kind === "new"
        ? newPromptEditorRef.current
        : pendingRef.current?.editor() ?? null;
    },
  }), [newPromptEditorRef, state.activeBuffer.kind]);

  const preferredQueueFocus = useMemo(() => {
    const match = resolveLastSubmittedRow(getLastSubmittedPending(sessionId), queue, drafts);
    return match?.kind === "queued" ? match.row.id : null;
  }, [drafts, queue, sessionId]);

  const onCanvasKeyDownCapture = useWorkbenchCanvasKeys({
    sessionId, queue, drafts, activeBuffer: state.activeBuffer, newPrompt,
    chordTrackerRef, openBuffer: state.openBuffer,
    onSaveAndClose: actions.handleSaveAndClose, onDiscard: actions.handleDiscard,
  });

  const bufferKey = state.activeBuffer.kind === "new"
    ? "new"
    : state.activeBuffer.kind === "new-draft"
    ? "new-draft"
    : `${state.activeBuffer.kind}:${state.activeBuffer.cmid ?? state.activeBuffer.id}`;

  return (
    <>
      <DesktopPendingPanels
        sessionId={sessionId} status={status} queue={queue} drafts={drafts}
        emptyDraftOpen={state.emptyDraftOpen} nextAgentContext={nextAgentContext}
        commands={commands} activeBuffer={state.activeBuffer}
        resolvedRowId={state.activeBuffer.kind === "queued" || state.activeBuffer.kind === "draft"
          ? (state.resolved?.row.id ?? state.activeBuffer.id)
          : null}
        preferredQueueFocus={preferredQueueFocus} onMoveDraft={onMoveDraft}
        onScheduleDraft={onScheduleDraft}
        onEditRow={(kind, msg, entryMode) =>
          state.openBuffer({
            kind, id: msg.id, ...(msg.cmid ? { cmid: msg.cmid } : {}),
            origin: "list", entryMode,
          })}
        onNewDraft={() => {
          state.setEmptyDraftOpen(false);
          state.openBuffer({ kind: "new-draft", origin: "list", entryMode: "insert" });
        }}
      />
      {beforeCanvas}
      <DesktopWorkbenchCanvas
        composerPaperRef={composerPaperRef} pendingRef={pendingRef}
        activeEditorProxy={activeEditorProxy} sendButtonRef={sendButtonRef}
        sessionId={sessionId} status={status} handedOff={handedOff}
        overlayOpen={overlayOpen} currentTurnAvailable={currentTurnAvailable}
        commands={commands} activeBuffer={state.activeBuffer} bufferKey={bufferKey}
        resolved={state.resolved} bufferMeta={state.bufferMeta}
        setBufferMeta={state.setBufferMeta} setActiveBuffer={state.setActiveBuffer}
        closeBuffer={state.closeBuffer} newPrompt={newPrompt}
        newPromptCanvas={newPromptCanvas} actions={actions}
        onMoveDraft={onMoveDraft} onCanvasKeyDownCapture={onCanvasKeyDownCapture}
      />
      <DesktopWorkbenchDialogs
        sessionId={sessionId} status={status} paused={paused}
        currentTurnAvailable={currentTurnAvailable}
        queuedAhead={state.activeBuffer.kind === "queued" && state.resolved
          ? state.resolved.index
          : queue.length}
        activeBuffer={state.activeBuffer} deliveryAnchor={actions.deliveryAnchor}
        onCloseDelivery={() => {
          actions.closeDelivery();
          requestAnimationFrame(() => {
            activeEditorProxy.current?.focus();
            if (state.activeBuffer.kind !== "new" && state.activeBuffer.entryMode === "insert") {
              requestDesktopEditorInsert();
            }
          });
        }}
        onDeliverBuffer={actions.deliverBuffer} onStop={actions.onStop}
        pendingSwitch={state.pendingSwitch}
        onCloseSwitch={() => state.setPendingSwitch(null)}
        onConfirmSwitch={state.confirmSwitch}
      />
    </>
  );
}
