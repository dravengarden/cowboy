import { type MutableRefObject, type RefObject, useCallback, useEffect, useRef, useState } from "react";
import {
  type Attachment,
  filesToAttachments,
  fileToAttachment,
  imageTokensInText,
  pendingImageAttachment,
  promoteUnplacedImageTokens,
  reconcileDeletedInlineImages,
  settlePendingAttachments,
} from "../attachments";
import type { ComposerEditorHandle, ComposerEditorSelection } from "../composer/PlatformComposerEditor";
import { getInlineAttachment, registerInlineAttachment, seedInlineAttachments } from "../inlineImages";
import { pendingContentCleared } from "../PendingPanel";
import {
  clearPendingEdit,
  getPendingEdit,
  setPendingEdit,
  subscribePendingEditRestore,
} from "../pendingEditStore";
import {
  copyDocumentToSession,
  editDraft,
  editQueued,
  notify,
  type QueuedMessage,
  setQueueEditing,
} from "../store";
import { isImeComposing } from "./vim/imeStatusStore";

const AUTOSAVE_DEBOUNCE_MS = 600;

export interface PendingBufferState {
  sessionId: string;
  kind: "queued" | "draft" | "new-draft";
  row: QueuedMessage | null;
  editorRef: RefObject<ComposerEditorHandle | null>;
  onMaterializedDraft: (rowId: string, cmid?: string) => void;
  onSourceVanished: (recoveredDraft?: { id?: string; cmid: string }) => void;
}

function usePendingBufferEffects({
  sessionId,
  kind,
  row,
  editorRef,
  textRef,
  attachmentsRef,
  committedTextRef,
  committedAttachmentsRef,
  setAttachments,
  onSourceVanished,
}: {
  sessionId: string;
  kind: "queued" | "draft" | "new-draft";
  row: QueuedMessage | null;
  editorRef: RefObject<ComposerEditorHandle | null>;
  textRef: MutableRefObject<string>;
  attachmentsRef: MutableRefObject<Attachment[]>;
  committedTextRef: MutableRefObject<string>;
  committedAttachmentsRef: MutableRefObject<Attachment[]>;
  setAttachments: React.Dispatch<React.SetStateAction<Attachment[]>>;
  onSourceVanished: (recovered?: { id?: string; cmid: string }) => void;
}): void {
  const isNewDraft = kind === "new-draft";
  const initialRowRef = useRef(row);
  const vanishedHandledRef = useRef(false);

  const holdId = kind === "queued" && row && row.status === undefined ? row.id : null;
  useEffect(() => {
    if (holdId === null) return undefined;
    setQueueEditing(sessionId, holdId);
    return () => setQueueEditing(sessionId, null);
  }, [holdId, sessionId]);

  const rowId = row?.id ?? null;
  useEffect(() => {
    if (isNewDraft || rowId === null) return undefined;
    return subscribePendingEditRestore(sessionId, kind, rowId, (restored) => {
      seedInlineAttachments(restored.attachments);
      const ids = new Set(restored.attachments.map((a) => a.id));
      for (const token of imageTokensInText(textRef.current)) {
        if (!ids.has(token.id) && getInlineAttachment(token.id) === undefined) {
          editorRef.current?.deleteImage(token.id);
        }
      }
      setAttachments((current) => {
        const held = new Set(current.map((a) => a.id));
        const missing = restored.attachments.filter((a) => !held.has(a.id));
        const next = missing.length === 0 ? current : [...current, ...missing];
        attachmentsRef.current = next;
        return next;
      });
      editorRef.current?.refreshImages();
    });
  }, [attachmentsRef, editorRef, isNewDraft, kind, rowId, sessionId, setAttachments, textRef]);

  useEffect(() => {
    if (isNewDraft || row !== null || vanishedHandledRef.current || !initialRowRef.current) return;
    vanishedHandledRef.current = true;
    const live = editorRef.current?.getValue() ?? textRef.current;
    const atts = attachmentsRef.current.filter((a) => !a.pending);
    const hasChanges = live.trimEnd() !== committedTextRef.current.trimEnd() ||
      atts.length !== committedAttachmentsRef.current.length;
    if (!hasChanges || pendingContentCleared(live, atts)) {
      clearPendingEdit(sessionId, kind, initialRowRef.current.id);
      notify("This message was already sent or removed.", "info");
      onSourceVanished();
      return;
    }
    void copyDocumentToSession(sessionId, live, atts).then((receipt) => {
      clearPendingEdit(sessionId, kind, initialRowRef.current!.id);
      notify("Original message was sent; editing your modified copy as a draft.", "warning");
      onSourceVanished({ cmid: receipt.cmid });
    }).catch((error: Error) => {
      notify(error.message, "error");
      onSourceVanished();
    });
  }, [
    attachmentsRef,
    committedAttachmentsRef,
    committedTextRef,
    editorRef,
    isNewDraft,
    kind,
    onSourceVanished,
    row,
    sessionId,
    textRef,
  ]);
}

function usePendingBufferFiles({
  editorRef,
  textRef,
  attachmentsRef,
  setAttachments,
  onMutate,
}: {
  editorRef: RefObject<ComposerEditorHandle | null>;
  textRef: MutableRefObject<string>;
  attachmentsRef: MutableRefObject<Attachment[]>;
  setAttachments: React.Dispatch<React.SetStateAction<Attachment[]>>;
  onMutate: (nextText: string, nextAtts: Attachment[]) => void;
}) {
  const addFiles = useCallback((
    files: File[],
    options: { preserveFocus?: boolean; selection?: ComposerEditorSelection } = {},
  ): void => {
    if (files.length === 0) return;
    const images = files.filter((f) => (f.type || "").startsWith("image/"));
    const rest = files.filter((f) => !(f.type || "").startsWith("image/"));
    if (images.length > 0) {
      const pending = images.map((f) => pendingImageAttachment(f));
      pending.forEach(registerInlineAttachment);
      setAttachments((prev) => {
        const next = [...prev, ...pending];
        attachmentsRef.current = next;
        return next;
      });
      editorRef.current?.insertImages(pending, options.selection);
      void Promise.allSettled(
        images.map((f, idx) => fileToAttachment(f, pending[idx]!.id)),
      ).then((settled) => {
        const done = settled.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
        done.forEach(registerInlineAttachment);
        setAttachments((current) => {
          const next = settlePendingAttachments(current, pending, done);
          attachmentsRef.current = next;
          onMutate(textRef.current, next);
          return next;
        });
        editorRef.current?.refreshImages();
      });
    }
    if (rest.length > 0) {
      void filesToAttachments(rest).then((added) => {
        if (added.length === 0) return;
        setAttachments((current) => {
          const next = [...current, ...added];
          attachmentsRef.current = next;
          onMutate(textRef.current, next);
          return next;
        });
      });
    }
  }, [attachmentsRef, editorRef, onMutate, setAttachments, textRef]);

  const removeAttachment = useCallback((id: string): void => {
    setAttachments((current) => {
      const next = current.filter((a) => a.id !== id);
      attachmentsRef.current = next;
      onMutate(textRef.current, next);
      return next;
    });
  }, [attachmentsRef, onMutate, setAttachments, textRef]);

  return { addFiles, removeAttachment };
}

async function commitPendingBufferSave({
  sessionId,
  kind,
  target,
  liveText,
  liveAtts,
  recomputeDirty,
  committedTextRef,
  committedAttachmentsRef,
  setDirty,
  flashSaved,
  silent,
  onMaterializedDraft,
}: {
  sessionId: string;
  kind: "queued" | "draft" | "new-draft";
  target: QueuedMessage | null;
  liveText: string;
  liveAtts: Attachment[];
  recomputeDirty: (text: string, atts: readonly Attachment[]) => boolean;
  committedTextRef: MutableRefObject<string>;
  committedAttachmentsRef: MutableRefObject<Attachment[]>;
  setDirty: React.Dispatch<React.SetStateAction<boolean>>;
  flashSaved: (label: string) => void;
  silent: boolean;
  onMaterializedDraft: (rowId: string, cmid?: string) => void;
}): Promise<boolean> {
  if (kind === "new-draft") {
    const receipt = await copyDocumentToSession(sessionId, liveText, liveAtts);
    committedTextRef.current = liveText;
    committedAttachmentsRef.current = liveAtts;
    setDirty(false);
    if (!silent) flashSaved("Saved");
    onMaterializedDraft("", receipt.cmid);
    return true;
  }
  if (!target || !recomputeDirty(liveText, liveAtts)) return Boolean(target);
  if (kind === "draft") await editDraft(sessionId, target.id, liveText, liveAtts);
  else await editQueued(sessionId, target.id, liveText, liveAtts);
  clearPendingEdit(sessionId, kind, target.id);
  committedTextRef.current = liveText;
  committedAttachmentsRef.current = liveAtts;
  setDirty(false);
  if (!silent) flashSaved("Saved");
  return true;
}

function usePendingBufferSave({
  sessionId,
  kind,
  autosaveEligible,
  editorRef,
  rowRef,
  textRef,
  attachmentsRef,
  committedTextRef,
  committedAttachmentsRef,
  setDirty,
  onMaterializedDraft,
}: {
  sessionId: string;
  kind: "queued" | "draft" | "new-draft";
  autosaveEligible: boolean;
  editorRef: RefObject<ComposerEditorHandle | null>;
  rowRef: MutableRefObject<QueuedMessage | null>;
  textRef: MutableRefObject<string>;
  attachmentsRef: MutableRefObject<Attachment[]>;
  committedTextRef: MutableRefObject<string>;
  committedAttachmentsRef: MutableRefObject<Attachment[]>;
  setDirty: React.Dispatch<React.SetStateAction<boolean>>;
  onMaterializedDraft: (rowId: string, cmid?: string) => void;
}) {
  const [saving, setSaving] = useState(false);
  const [savedLabel, setSavedLabel] = useState<string | null>(null);
  const inFlightRef = useRef<Promise<boolean> | null>(null);
  const queuedSaveRef = useRef(false);
  const autosaveTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const savedBadgeTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const recomputeDirty = useCallback((nextText: string, nextAtts: readonly Attachment[]): boolean => {
    const isDirty = nextText.trimEnd() !== committedTextRef.current.trimEnd() ||
      nextAtts.length !== committedAttachmentsRef.current.length ||
      nextAtts.some((a, i) => a.id !== committedAttachmentsRef.current[i]?.id);
    setDirty(isDirty);
    return isDirty;
  }, [committedAttachmentsRef, committedTextRef, setDirty]);

  const flashSaved = useCallback((label: string): void => {
    setSavedLabel(label);
    if (savedBadgeTimerRef.current !== undefined) clearTimeout(savedBadgeTimerRef.current);
    savedBadgeTimerRef.current = setTimeout(() => setSavedLabel(null), 1800);
  }, []);

  const persistLocal = useCallback((nextText: string, nextAtts: Attachment[]): void => {
    const current = rowRef.current;
    if (kind === "new-draft" || !current) return;
    setPendingEdit({
      sessionId,
      kind,
      id: current.id,
      text: nextText,
      attachments: nextAtts.filter((a) => !a.pending),
      baseText: committedTextRef.current,
      baseAttachments: committedAttachmentsRef.current,
    });
  }, [committedAttachmentsRef, committedTextRef, kind, rowRef, sessionId]);

  const saveNow = useCallback(async (opts?: { silent?: boolean }): Promise<boolean> => {
    if (inFlightRef.current) {
      queuedSaveRef.current = true;
      return inFlightRef.current;
    }
    const run = async (): Promise<boolean> => {
      const liveText = editorRef.current?.getValue() ?? textRef.current;
      textRef.current = liveText;
      const liveAtts = attachmentsRef.current;
      if (liveAtts.some((a) => a.pending) || pendingContentCleared(liveText, liveAtts)) return false;
      setSaving(true);
      try {
        return await commitPendingBufferSave({
          sessionId,
          kind,
          target: rowRef.current,
          liveText,
          liveAtts,
          recomputeDirty,
          committedTextRef,
          committedAttachmentsRef,
          setDirty,
          flashSaved,
          silent: opts?.silent === true,
          onMaterializedDraft,
        });
      } finally {
        setSaving(false);
      }
    };
    const promise = run().finally(() => {
      inFlightRef.current = null;
      if (queuedSaveRef.current) {
        queuedSaveRef.current = false;
        void saveNow({ silent: true }).catch(() => undefined);
      }
    });
    inFlightRef.current = promise;
    return promise;
  }, [
    attachmentsRef,
    committedAttachmentsRef,
    committedTextRef,
    editorRef,
    flashSaved,
    kind,
    onMaterializedDraft,
    recomputeDirty,
    rowRef,
    sessionId,
    setDirty,
    textRef,
  ]);

  const scheduleAutosave = useCallback((): void => {
    if (!autosaveEligible) return;
    if (autosaveTimerRef.current !== undefined) clearTimeout(autosaveTimerRef.current);
    autosaveTimerRef.current = setTimeout(() => {
      autosaveTimerRef.current = undefined;
      if (isImeComposing() || (kind === "draft" && rowRef.current?.schedule)) return;
      void saveNow({ silent: true }).catch(() => undefined);
    }, AUTOSAVE_DEBOUNCE_MS);
  }, [autosaveEligible, kind, rowRef, saveNow]);

  const cancelTimers = useCallback((): void => {
    if (autosaveTimerRef.current !== undefined) clearTimeout(autosaveTimerRef.current);
    if (savedBadgeTimerRef.current !== undefined) clearTimeout(savedBadgeTimerRef.current);
  }, []);

  useEffect(() => cancelTimers, [cancelTimers]);

  return {
    saving,
    savedLabel,
    recomputeDirty,
    persistLocal,
    saveNow,
    scheduleAutosave,
    cancelTimers,
  };
}

export function usePendingBufferController({
  sessionId,
  kind,
  row,
  editorRef,
  onMaterializedDraft,
  onSourceVanished,
}: PendingBufferState) {
  const isNewDraft = kind === "new-draft";
  const recovery = row && kind !== "new-draft" ? getPendingEdit(sessionId, kind, row.id) : null;
  const seedAttachments = recovery?.attachments ?? row?.attachments ?? [];
  const seedText = promoteUnplacedImageTokens(recovery?.text ?? row?.text ?? "", seedAttachments);
  const [mountSeed] = useState(seedText);
  const [attachments, setAttachments] = useState<Attachment[]>(() => {
    seedInlineAttachments(seedAttachments);
    return seedAttachments;
  });
  const [dirty, setDirty] = useState(() =>
    recovery !== null &&
    (recovery.text !== (row?.text ?? "") ||
      recovery.attachments.length !== (row?.attachments.length ?? 0))
  );

  const textRef = useRef(seedText);
  const attachmentsRef = useRef(seedAttachments);
  const committedTextRef = useRef(row?.text ?? "");
  const committedAttachmentsRef = useRef(row?.attachments ?? []);
  const rowRef = useRef(row);
  rowRef.current = row;

  const attachmentsPending = attachments.some((a) => a.pending);
  const sendable = !attachmentsPending && !pendingContentCleared(textRef.current, attachments);
  const scheduled = kind === "draft" && Boolean(row?.schedule);
  const autosaveEligible = (kind === "draft" && !scheduled) || isNewDraft;

  usePendingBufferEffects({
    sessionId,
    kind,
    row,
    editorRef,
    textRef,
    attachmentsRef,
    committedTextRef,
    committedAttachmentsRef,
    setAttachments,
    onSourceVanished,
  });

  const saver = usePendingBufferSave({
    sessionId,
    kind,
    autosaveEligible,
    editorRef,
    rowRef,
    textRef,
    attachmentsRef,
    committedTextRef,
    committedAttachmentsRef,
    setDirty,
    onMaterializedDraft,
  });

  const onMutate = useCallback((nextText: string, nextAtts: Attachment[]): void => {
    saver.recomputeDirty(nextText, nextAtts);
    saver.persistLocal(nextText, nextAtts);
    saver.scheduleAutosave();
  }, [saver]);

  const updateText = useCallback((next: string): void => {
    const prev = textRef.current;
    textRef.current = next;
    const reconciled = reconcileDeletedInlineImages(
      prev,
      next,
      attachmentsRef.current,
      getInlineAttachment,
    );
    if (reconciled !== attachmentsRef.current) {
      attachmentsRef.current = reconciled;
      setAttachments(reconciled);
    }
    onMutate(next, attachmentsRef.current);
  }, [onMutate]);

  const { addFiles, removeAttachment } = usePendingBufferFiles({
    editorRef,
    textRef,
    attachmentsRef,
    setAttachments,
    onMutate,
  });

  const discard = useCallback((): void => {
    saver.cancelTimers();
    const current = rowRef.current;
    if (kind !== "new-draft" && current) clearPendingEdit(sessionId, kind, current.id);
    if (kind === "queued") setQueueEditing(sessionId, null);
    setDirty(false);
  }, [kind, saver, sessionId]);

  return {
    mountSeed,
    textRef,
    attachments,
    attachmentsRef,
    attachmentsPending,
    sendable,
    dirty,
    saving: saver.saving,
    savedLabel: saver.savedLabel,
    scheduled,
    updateText,
    addFiles,
    removeAttachment,
    saveNow: saver.saveNow,
    discard,
  };
}
