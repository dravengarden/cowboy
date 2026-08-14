import {
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { flushSync } from "react-dom";
import {
  type Attachment,
  filesToAttachments,
  fileToAttachment,
  pendingImageAttachment,
  promoteUnplacedImageTokens,
  reconcileDeletedInlineImages,
  settlePendingAttachments,
} from "../attachments";
import { getDraft, setDraft } from "../draftStore";
import {
  registerInlineAttachment,
  seedInlineAttachments,
} from "../inlineImages";
import type {
  ComposerEditorHandle,
  ComposerEditorSelection,
} from "./PlatformComposerEditor";
import type { Delivery } from "../protocol";
import {
  addDraft,
  forcePrompt,
  frontPrompt,
  scheduleDraft,
  submitPrompt,
} from "../store";
import { haptic } from "../haptic";
import {
  type NativeClipboardImagePasteRequest,
  runNativeClipboardImagePaste,
} from "./nativeClipboardImagePaste";
import { prepareUserPrompt } from "./slashCommandIntent";

export interface ComposerDraftController {
  text: string;
  setText: (text: string) => void;
  attachments: Attachment[];
  setAttachments: React.Dispatch<React.SetStateAction<Attachment[]>>;
  initialText: React.MutableRefObject<string>;
  sendable: boolean;
  addFiles: (
    files: File[],
    options?: {
      preserveFocus?: boolean;
      selection?: ComposerEditorSelection;
    },
  ) => void;
  pasteClipboardImages: (
    request: NativeClipboardImagePasteRequest,
  ) => Promise<void>;
  removeAttachment: (id: string) => void;
  clear: () => void;
  submit: () => boolean;
  submitTracked: () => Promise<void> | null;
  force: () => boolean;
  forceTracked: () => Promise<void> | null;
  jumpToFront: (queueLength: number) => boolean;
  jumpToFrontTracked: (queueLength: number) => Promise<void> | null;
  saveAsDraft: () => boolean;
  saveAsDraftTracked: () => Promise<void> | null;
  scheduleNew: (fireAtMs: number, delivery: Delivery) => boolean;
}

export interface ComposerDraftControllerOptions {
  /**
   * Native text controls need React to mirror every input value. Desktop CM6
   * owns its document, so mirroring each composition update would only rerender
   * the whole Prompt workspace and delay the editor's next paint.
   */
  mirrorTextInReact?: boolean;
}

// Owns the local, not-yet-sent prompt. This is product behaviour shared by the
// Desktop and Mobile composer shells: persistence, attachment staging and the
// imperative clear required by the uncontrolled CM6 editor all live here.
export function useComposerDraftController(
  sessionId: string,
  editorRef: RefObject<ComposerEditorHandle | null>,
  { mirrorTextInReact = true }: ComposerDraftControllerOptions = {},
): ComposerDraftController {
  const seed = useRef(getDraft(sessionId)).current;
  const seededText = useRef(
    promoteUnplacedImageTokens(seed.text, seed.attachments),
  ).current;
  const [textState, setTextState] = useState(seededText);
  const textRef = useRef(seededText);
  const initialText = useRef(seededText);
  const [hasText, setHasText] = useState(seededText.trim().length > 0);
  const attachmentsRef = useRef(seed.attachments);
  // A native editor submit and a toolbar activation can reach this controller
  // in the same browser task. Claim the commit synchronously so one physical
  // action can mint at most one cmid before React applies the cleared state.
  const committingRef = useRef(false);
  const [attachments, setAttachmentsState] = useState<Attachment[]>(() => {
    seedInlineAttachments(seed.attachments);
    return seed.attachments;
  });

  const setAttachments = useCallback<React.Dispatch<React.SetStateAction<Attachment[]>>>(
    (update): void => {
      const current = attachmentsRef.current;
      const next = typeof update === "function" ? update(current) : update;
      // Keep the imperative editor path authoritative immediately. React may
      // defer the state commit, but a same-gesture paste can emit CM changes
      // before that commit and must reconcile against the new attachments.
      attachmentsRef.current = next;
      setAttachmentsState(next);
    },
    [],
  );

  const setText = useCallback((next: string): void => {
    const previous = textRef.current;
    textRef.current = next;
    const currentAttachments = attachmentsRef.current;
    const reconciled = reconcileDeletedInlineImages(
      previous,
      next,
      currentAttachments,
    );
    if (reconciled !== currentAttachments) {
      attachmentsRef.current = reconciled;
      setAttachmentsState(reconciled);
    }
    const nextHasText = next.trim().length > 0;
    setHasText((current) => current === nextHasText ? current : nextHasText);
    if (mirrorTextInReact) {
      setTextState(next);
    } else if (!reconciled.some((attachment) => attachment.pending)) {
      // CM6 is uncontrolled and the local draft store already debounces disk
      // I/O. Update that store directly without routing every keystroke through
      // React; refs remain authoritative for submit/park/schedule actions.
      setDraft(sessionId, { text: next, attachments: reconciled });
    }
  }, [mirrorTextInReact, sessionId]);

  useEffect(() => {
    // A same-gesture paste placeholder has only a local object URL and an empty
    // wire block. Keep the last durable draft until encoding completes.
    if (attachments.some((attachment) => attachment.pending)) return;
    setDraft(sessionId, { text: textRef.current, attachments });
  }, [sessionId, textState, attachments]);

  // Desktop reads the live ref whenever another meaningful state transition
  // renders the shell. Mobile/native controls consume the reactive mirror.
  const text = mirrorTextInReact ? textState : textRef.current;

  const addFiles = (
    files: File[],
    options: {
      preserveFocus?: boolean;
      selection?: ComposerEditorSelection;
    } = {},
  ): void => {
    if (files.length === 0) return;
    if (options.preserveFocus) {
      const images = files.filter((file) => (file.type || "").startsWith("image/"));
      const rest = files.filter((file) => !(file.type || "").startsWith("image/"));
      if (images.length > 0) {
        // UIKit only keeps the software keyboard when the focused-control
        // replacement is committed in the Paste gesture itself. Stage a local
        // object-URL preview synchronously; encode its ACP bytes afterward.
        const pending = images.map((file) => pendingImageAttachment(file));
        pending.forEach(registerInlineAttachment);
        setAttachments((previous) => [...previous, ...pending]);
        editorRef.current?.insertImages(pending, options.selection);
        void Promise.allSettled(
          images.map((file, index) => fileToAttachment(file, pending[index]!.id)),
        ).then((settled) => {
          const completed = settled.flatMap((result) =>
            result.status === "fulfilled" ? [result.value] : []
          );
          completed.forEach(registerInlineAttachment);
          const completedById = new Map(completed.map((item) => [item.id, item]));
          const failed = pending.filter((item) => !completedById.has(item.id));
          failed.forEach((item) => editorRef.current?.deleteImage(item.id));
          setAttachments((current) =>
            settlePendingAttachments(current, pending, completed)
          );
          editorRef.current?.refreshImages();
          pending.forEach((item) => {
            if (item.previewUrl?.startsWith("blob:")) URL.revokeObjectURL(item.previewUrl);
          });
        });
      }
      if (rest.length === 0) return;
      files = rest;
    }
    void filesToAttachments(files).then((added) => {
      if (added.length === 0) return;
      added.forEach(registerInlineAttachment);
      setAttachments((previous) => [...previous, ...added]);
      editorRef.current?.insertImages(added);
      if (options.preserveFocus) {
        // In compact touch mode, inserting the first image atomically promotes
        // the focused native textarea to CM6. Transfer focus once after React
        // commits that replacement so UIKit keeps the existing keyboard open.
        requestAnimationFrame(() => editorRef.current?.focus());
      }
    });
  };

  const pasteClipboardImages = (
    request: NativeClipboardImagePasteRequest,
  ): Promise<void> =>
    runNativeClipboardImagePaste(request, {
      stage: (pending, selection): void => {
        pending.forEach(registerInlineAttachment);
        const editor = editorRef.current;
        flushSync(() => {
          setAttachments((previous) => [...previous, ...pending]);
          editor?.insertImages(pending, selection);
        });
      },
      settle: (pending, completed): void => {
        completed.forEach(registerInlineAttachment);
        const completedById = new Map(
          completed.map((attachment) => [attachment.id, attachment]),
        );
        const editor = editorRef.current;
        pending.filter((attachment) => !completedById.has(attachment.id))
          .forEach((attachment) => editor?.deleteImage(attachment.id));
        setAttachments((current) =>
          settlePendingAttachments(current, pending, completed)
        );
        editor?.refreshImages();
      },
    });

  const removeAttachment = (id: string): void => {
    setAttachments((previous) =>
      previous.filter((attachment) => attachment.id !== id)
    );
  };

  const clear = (): void => {
    // ComposerEditor is intentionally uncontrolled. Clearing React state alone
    // would leave its document visible and feeding `value` back would break IME.
    editorRef.current?.clear();
    textRef.current = "";
    setTextState("");
    setHasText(false);
    attachmentsRef.current = [];
    setAttachmentsState([]);
    // Page View can intentionally unmount the composer immediately after send.
    // Persist the cleared value synchronously so a later remount cannot restore
    // the pre-send draft before React's effects get a chance to run.
    setDraft(sessionId, { text: "", attachments: [] });
  };

  const sendable = (hasText || attachments.length > 0) &&
    !attachments.some((attachment) => attachment.pending);
  const commitTracked = (
    action: () => Promise<void>,
    feedback = true,
  ): Promise<void> | null => {
    if (!sendable || committingRef.current) return null;
    committingRef.current = true;
    try {
      if (feedback) haptic();
      const confirmation = action();
      clear();
      return confirmation;
    } finally {
      // React has now received the clear updates. The latch only arbitrates
      // competing delivery entry points from this one physical action; it must
      // not prevent the user composing a genuinely new prompt afterward.
      globalThis.queueMicrotask(() => {
        committingRef.current = false;
      });
    }
  };
  const commit = (action: () => Promise<void>, feedback = true): boolean => {
    const confirmation = commitTracked(action, feedback);
    if (confirmation === null) return false;
    // Keyboard/editor submission has no button to own the pending lifecycle.
    // The optimistic row remains visible; keep a rejected acknowledgement from
    // becoming an unhandled promise while the store's timeout marks it failed.
    void confirmation.catch(() => undefined);
    return true;
  };
  const preparedText = (): string =>
    prepareUserPrompt(
      textRef.current.trimEnd(),
      editorRef.current?.consumeSelectedSlashCommand() ?? null,
    );

  return {
    text,
    setText,
    attachments,
    setAttachments,
    initialText,
    sendable,
    addFiles,
    pasteClipboardImages,
    removeAttachment,
    clear,
    submit: () =>
      commit(() => submitPrompt(sessionId, preparedText(), attachments)),
    submitTracked: () =>
      commitTracked(() => submitPrompt(sessionId, preparedText(), attachments)),
    force: () =>
      commit(() => forcePrompt(sessionId, preparedText(), attachments)),
    forceTracked: () =>
      commitTracked(() => forcePrompt(sessionId, preparedText(), attachments)),
    jumpToFront: (queueLength) =>
      queueLength > 0 &&
      commit(() => frontPrompt(sessionId, preparedText(), attachments)),
    jumpToFrontTracked: (queueLength) =>
      queueLength > 0
        ? commitTracked(() => frontPrompt(sessionId, preparedText(), attachments))
        : null,
    // Parking/scheduling is deliberate state management rather than a send
    // gesture, so preserve the existing no-haptic behaviour.
    saveAsDraft: () =>
      commit(
        () => addDraft(sessionId, textRef.current.trimEnd(), attachments),
        false,
      ),
    saveAsDraftTracked: () =>
      commitTracked(
        () => addDraft(sessionId, textRef.current.trimEnd(), attachments),
        false,
      ),
    scheduleNew: (fireAtMs, delivery) =>
      commit(
        () => {
          scheduleDraft(sessionId, {
            text: preparedText(),
            attachments,
            fireAtMs,
            delivery,
          });
          return Promise.resolve();
        },
        false,
      ),
  };
}
