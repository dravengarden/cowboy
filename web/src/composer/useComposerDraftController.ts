import {
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  type Attachment,
  filesToAttachments,
  fileToAttachment,
  pendingImageAttachment,
  promoteUnplacedImageTokens,
  reconcileDeletedInlineImages,
} from "../attachments";
import { getDraft, setDraft } from "../draftStore";
import {
  registerInlineAttachment,
  seedInlineAttachments,
} from "../inlineImages";
import type { ComposerEditorHandle } from "./PlatformComposerEditor";
import type { Delivery } from "../protocol";
import {
  addDraft,
  forcePrompt,
  frontPrompt,
  scheduleDraft,
  submitPrompt,
} from "../store";
import { haptic } from "../haptic";
import { prepareUserPrompt } from "./slashCommandIntent";

export interface ComposerDraftController {
  text: string;
  setText: (text: string) => void;
  attachments: Attachment[];
  setAttachments: React.Dispatch<React.SetStateAction<Attachment[]>>;
  initialText: React.MutableRefObject<string>;
  sendable: boolean;
  addFiles: (files: File[], options?: { preserveFocus?: boolean }) => void;
  removeAttachment: (id: string) => void;
  clear: () => void;
  submit: () => boolean;
  force: () => boolean;
  jumpToFront: (queueLength: number) => boolean;
  saveAsDraft: () => boolean;
  scheduleNew: (fireAtMs: number, delivery: Delivery) => boolean;
}

// Owns the local, not-yet-sent prompt. This is product behaviour shared by the
// Desktop and Mobile composer shells: persistence, attachment staging and the
// imperative clear required by the uncontrolled CM6 editor all live here.
export function useComposerDraftController(
  sessionId: string,
  editorRef: RefObject<ComposerEditorHandle | null>,
): ComposerDraftController {
  const seed = useRef(getDraft(sessionId)).current;
  const seededText = useRef(
    promoteUnplacedImageTokens(seed.text, seed.attachments),
  ).current;
  const [text, setTextState] = useState(seededText);
  const textRef = useRef(seededText);
  const initialText = useRef(seededText);
  const [attachments, setAttachments] = useState<Attachment[]>(() => {
    seedInlineAttachments(seed.attachments);
    return seed.attachments;
  });

  const setText = (next: string): void => {
    const previous = textRef.current;
    textRef.current = next;
    setAttachments((current) =>
      reconcileDeletedInlineImages(previous, next, current)
    );
    setTextState(next);
  };

  useEffect(() => {
    // A same-gesture paste placeholder has only a local object URL and an empty
    // wire block. Keep the last durable draft until encoding completes.
    if (attachments.some((attachment) => attachment.pending)) return;
    setDraft(sessionId, { text, attachments });
  }, [sessionId, text, attachments]);

  const addFiles = (
    files: File[],
    options: { preserveFocus?: boolean } = {},
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
        editorRef.current?.insertImages(pending);
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
            current.flatMap((item) => {
              if (!item.pending) return [item];
              const replacement = completedById.get(item.id);
              return replacement ? [replacement] : [];
            })
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
    setAttachments([]);
    // Page View can intentionally unmount the composer immediately after send.
    // Persist the cleared value synchronously so a later remount cannot restore
    // the pre-send draft before React's effects get a chance to run.
    setDraft(sessionId, { text: "", attachments: [] });
  };

  const sendable = (text.trim().length > 0 || attachments.length > 0) &&
    !attachments.some((attachment) => attachment.pending);
  const commit = (action: () => void, feedback = true): boolean => {
    if (!sendable) return false;
    if (feedback) haptic();
    action();
    clear();
    return true;
  };
  const preparedText = (): string =>
    prepareUserPrompt(
      text.trimEnd(),
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
    removeAttachment,
    clear,
    submit: () =>
      commit(() => submitPrompt(sessionId, preparedText(), attachments)),
    force: () =>
      commit(() => forcePrompt(sessionId, preparedText(), attachments)),
    jumpToFront: (queueLength) =>
      queueLength > 0 &&
      commit(() => frontPrompt(sessionId, preparedText(), attachments)),
    // Parking/scheduling is deliberate state management rather than a send
    // gesture, so preserve the existing no-haptic behaviour.
    saveAsDraft: () =>
      commit(() => addDraft(sessionId, text.trimEnd(), attachments), false),
    scheduleNew: (fireAtMs, delivery) =>
      commit(
        () =>
          scheduleDraft(sessionId, {
            text: preparedText(),
            attachments,
            fireAtMs,
            delivery,
          }),
        false,
      ),
  };
}
