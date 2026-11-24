import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import { alpha, Box } from "@mui/material";
import { attachmentTrayForSurface } from "../composer/attachmentPresentation";
import { useComposerFileDrop } from "../composer/composerFileDrop";
import {
  type ComposerEditorHandle,
  PlatformComposerEditor,
} from "../composer/PlatformComposerEditor";
import { AttachmentPreviews } from "../PendingPanel";
import type { AvailableCommand } from "../protocol";
import { fireLabel } from "../scheduleTime";
import {
  type QueuedMessage,
  useQueueEditHoldStatus,
} from "../store";
import { isImeKeyEvent } from "../imeKey";
import {
  clearBufferVimMode,
  setActiveVimOwner,
  setBufferVimMode,
} from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { requestDesktopEditorInsert } from "./editorInsert";
import { DesktopPromptBufferBar } from "./DesktopPromptBufferBar";
import {
  type MaterializedCarry,
  usePendingBufferController,
} from "./usePendingBufferController";

export interface DesktopPendingBufferHandle {
  editor: () => ComposerEditorHandle | null;
  isDirty: () => boolean;
  isSendable: () => boolean;
  isEncoding: () => boolean;
  isSaving: () => boolean;
  isScheduled: () => boolean;
  getText: () => string;
  getAttachments: () => readonly import("../attachments").Attachment[];
  attach: () => void;
  save: (opts?: { silent?: boolean }) => Promise<boolean>;
  discard: () => void;
  flashDirtyHint: () => void;
}

export interface DesktopPendingBufferEditorProps {
  sessionId: string;
  kind: "queued" | "draft" | "new-draft";
  row: QueuedMessage | null;
  initialCarry?: MaterializedCarry | null | undefined;
  index?: number | undefined;
  total?: number | undefined;
  entryMode: "insert" | "normal";
  commands: () => AvailableCommand[];
  onStateChange: (snapshot: {
    sendable: boolean;
    dirty: boolean;
    saving: boolean;
    scheduled: boolean;
    holdStatus: import("../queueEditingHold").QueueEditHoldStatus;
  }) => void;
  onMaterializedDraft: (rowId: string, cmid?: string, carry?: MaterializedCarry) => void;
  onSourceVanished: (recovered?: { id?: string; cmid: string }) => void;
  onEscape: () => void;
  onBack: () => void;
  onDiscard: () => void;
  onDemoteToDraft?: (() => void) | undefined;
  onMoveToSession?: (() => void) | undefined;
  onPromoteToDocument?: (() => void) | undefined;
}

function usePendingBufferFocus({
  vimOwnerId,
  entryMode,
  vim,
  editorRef,
  onStateChange,
  stateSnapshot,
}: {
  vimOwnerId: string;
  entryMode: "insert" | "normal";
  vim: boolean;
  editorRef: React.RefObject<ComposerEditorHandle | null>;
  onStateChange: DesktopPendingBufferEditorProps["onStateChange"];
  stateSnapshot: Parameters<DesktopPendingBufferEditorProps["onStateChange"]>[0];
}) {
  const [flashHint, setFlashHint] = useState<string | null>(null);
  const flashTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const flashDirtyHint = useCallback((): void => {
    setFlashHint("Unsaved edits · Mod+Enter finish · Mod+S save · ZQ discard");
    if (flashTimerRef.current !== undefined) clearTimeout(flashTimerRef.current);
    flashTimerRef.current = setTimeout(() => setFlashHint(null), 2800);
  }, []);

  useEffect(() => () => {
    if (flashTimerRef.current !== undefined) clearTimeout(flashTimerRef.current);
  }, []);

  useEffect(() => {
    setActiveVimOwner(vimOwnerId);
    return () => {
      clearBufferVimMode(vimOwnerId);
      setActiveVimOwner("default");
    };
  }, [vimOwnerId]);

  useEffect(() => {
    let frame2 = 0;
    const frame1 = requestAnimationFrame(() => {
      editorRef.current?.focusEnd();
      if (entryMode === "insert" && vim) {
        frame2 = requestAnimationFrame(() => requestDesktopEditorInsert());
      }
    });
    return () => {
      cancelAnimationFrame(frame1);
      if (frame2) cancelAnimationFrame(frame2);
    };
  }, [editorRef, entryMode, vim]);

  useEffect(() => {
    onStateChange(stateSnapshot);
  }, [
    onStateChange,
    stateSnapshot.dirty,
    stateSnapshot.holdStatus,
    stateSnapshot.saving,
    stateSnapshot.scheduled,
    stateSnapshot.sendable,
  ]);

  return { flashHint, flashDirtyHint };
}

interface DesktopPendingBufferViewProps {
  barKind: "queued" | "draft" | "scheduled" | "new-draft";
  index?: number | undefined;
  total?: number | undefined;
  kind: "queued" | "draft" | "new-draft";
  sessionId: string;
  vim: boolean;
  vimOwnerId: string;
  entryMode: "insert" | "normal";
  commands: () => AvailableCommand[];
  editorRef: React.RefObject<ComposerEditorHandle | null>;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  ctrl: ReturnType<typeof usePendingBufferController>;
  fileDrop: ReturnType<typeof useComposerFileDrop>;
  holdStatus: import("../queueEditingHold").QueueEditHoldStatus;
  scheduleLabel?: string | undefined;
  flashHint: string | null;
  onEscape: () => void;
  onBack: () => void;
  onDiscard: () => void;
  onDemoteToDraft?: (() => void) | undefined;
  onMoveToSession?: (() => void) | undefined;
  onPromoteToDocument?: (() => void) | undefined;
}

function DesktopPendingBufferView({
  barKind, index, total, kind, sessionId, vim, vimOwnerId, entryMode, commands,
  editorRef, fileInputRef, ctrl, fileDrop, holdStatus, scheduleLabel, flashHint,
  onEscape, onBack, onDiscard, onDemoteToDraft, onMoveToSession, onPromoteToDocument,
}: DesktopPendingBufferViewProps): React.JSX.Element {
  const trayAttachments = attachmentTrayForSurface(
    ctrl.attachments,
    ctrl.textRef.current,
  );
  return (
    <Box
      data-desktop-active-editor="true"
      data-desktop-buffer-kind={barKind}
      {...fileDrop.handlers}
      onKeyDownCapture={(event): void => {
        if (event.key !== "Escape" || isImeKeyEvent(event.nativeEvent)) return;
        if (!editorRef.current?.escapeBelongsToApp()) return;
        event.preventDefault();
        event.stopPropagation();
        onEscape();
      }}
      sx={{
        position: "relative",
        flex: 1,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      }}
    >
      <input
        ref={fileInputRef}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          ctrl.addFiles(Array.from(e.target.files ?? []), { preserveFocus: true });
          e.target.value = "";
        }}
      />
      <DesktopPromptBufferBar
        kind={barKind}
        index={index}
        total={total}
        dirty={ctrl.dirty}
        saving={ctrl.saving}
        savedLabel={ctrl.savedLabel}
        holdStatus={kind === "queued" ? holdStatus : "none"}
        scheduleLabel={scheduleLabel}
        flashHint={flashHint}
        onBack={onBack}
        onDiscard={kind === "queued" || ctrl.scheduled ? onDiscard : undefined}
        onDemoteToDraft={kind === "queued" ? onDemoteToDraft : undefined}
        onMoveToSession={kind === "draft" ? onMoveToSession : undefined}
        onPromoteToDocument={kind === "draft" && !ctrl.scheduled ? onPromoteToDocument : undefined}
      />
      {fileDrop.active && (
        <Box
          data-composer-file-drop="true"
          aria-hidden
          sx={{
            position: "absolute",
            inset: 0,
            zIndex: 5,
            display: "grid",
            placeItems: "center",
            border: 2,
            borderStyle: "dashed",
            borderColor: "primary.main",
            bgcolor: (t) => alpha(t.palette.primary.main, 0.08),
            color: "primary.main",
            typography: "body2",
            fontWeight: 600,
            pointerEvents: "none",
          }}
        >
          Drop files to attach
        </Box>
      )}
      {trayAttachments.length > 0 && (
        <AttachmentPreviews
          attachments={trayAttachments}
          onRemove={ctrl.removeAttachment}
        />
      )}
      <Box sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlatformComposerEditor
          ref={editorRef}
          value={ctrl.mountSeed}
          onChange={ctrl.updateText}
          borderless
          fill
          flushRightScrollbar
          vim={vim}
          focusEndOnMount={entryMode === "insert" || !vim}
          onVimMode={(mode) => setBufferVimMode(vimOwnerId, mode)}
          sessionId={sessionId}
          commands={commands}
          placeholder={kind === "queued"
            ? "Edit queued message…"
            : "Write session draft…"}
          onPasteFiles={(files) => ctrl.addFiles(files, { preserveFocus: true })}
          onEscape={() => {
            onEscape();
            return true;
          }}
        />
      </Box>
    </Box>
  );
}

export const DesktopPendingBufferEditor = forwardRef<
  DesktopPendingBufferHandle,
  DesktopPendingBufferEditorProps
>(function DesktopPendingBufferEditor(props, ref): React.JSX.Element {
  const {
    sessionId,
    kind,
    row,
    initialCarry,
    index,
    total,
    entryMode,
    commands,
    onStateChange,
    onMaterializedDraft,
    onSourceVanished,
    onEscape,
    onBack,
    onDiscard,
    onDemoteToDraft,
    onMoveToSession,
    onPromoteToDocument,
  } = props;
  const vim = useVimSetting();
  const editorRef = useRef<ComposerEditorHandle | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const holdStatus = useQueueEditHoldStatus(sessionId);
  const vimOwnerId = `${kind}:${row?.id ?? "new"}`;

  const ctrl = usePendingBufferController({
    sessionId,
    kind,
    row,
    initialCarry,
    editorRef,
    onMaterializedDraft,
    onSourceVanished,
  });

  const fileDrop = useComposerFileDrop(
    true,
    (files) => ctrl.addFiles(files, { preserveFocus: true }),
  );

  const { flashHint, flashDirtyHint } = usePendingBufferFocus({
    vimOwnerId,
    entryMode,
    vim,
    editorRef,
    onStateChange,
    stateSnapshot: {
      sendable: ctrl.sendable,
      dirty: ctrl.dirty,
      saving: ctrl.saving,
      scheduled: ctrl.scheduled,
      holdStatus: kind === "queued" ? holdStatus : "none",
    },
  });

  useImperativeHandle(ref, () => ({
    editor: () => editorRef.current,
    isDirty: () => ctrl.dirty,
    isSendable: () => ctrl.sendable,
    isEncoding: () => ctrl.attachmentsPending,
    isSaving: () => ctrl.saving,
    isScheduled: () => ctrl.scheduled,
    getText: () => editorRef.current?.getValue() ?? ctrl.textRef.current,
    getAttachments: () => ctrl.attachmentsRef.current,
    attach: () => fileInputRef.current?.click(),
    save: (opts) => ctrl.saveNow(opts),
    discard: () => ctrl.discard(),
    flashDirtyHint,
  }), [ctrl, flashDirtyHint]);

  const barKind = kind === "queued"
    ? "queued"
    : ctrl.scheduled
    ? "scheduled"
    : kind === "new-draft"
    ? "new-draft"
    : "draft";
  const scheduleLabel = row?.schedule
    ? `${fireLabel(row.schedule.fire_at_ms)} · ${row.schedule.delivery === "front" ? "Queue first" : "Next turn"}`
    : undefined;

  return (
    <DesktopPendingBufferView
      barKind={barKind}
      index={index}
      total={total}
      kind={kind}
      sessionId={sessionId}
      vim={vim}
      vimOwnerId={vimOwnerId}
      entryMode={entryMode}
      commands={commands}
      editorRef={editorRef}
      fileInputRef={fileInputRef}
      ctrl={ctrl}
      fileDrop={fileDrop}
      holdStatus={holdStatus}
      scheduleLabel={scheduleLabel}
      flashHint={flashHint}
      onEscape={onEscape}
      onBack={onBack}
      onDiscard={onDiscard}
      onDemoteToDraft={onDemoteToDraft}
      onMoveToSession={onMoveToSession}
      onPromoteToDocument={onPromoteToDocument}
    />
  );
});
