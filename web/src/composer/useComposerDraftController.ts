import {
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  type Attachment,
  filesToAttachments,
  reconcileDeletedInlineImages,
  stripImageTokens,
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
  addFiles: (files: File[]) => void;
  removeAttachment: (id: string) => void;
  demoteInlineImages: () => string;
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
  const [text, setTextState] = useState(seed.text);
  const textRef = useRef(seed.text);
  const initialText = useRef(seed.text);
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
    setDraft(sessionId, { text, attachments });
  }, [sessionId, text, attachments]);

  const addFiles = (files: File[]): void => {
    if (files.length === 0) return;
    void filesToAttachments(files).then((added) => {
      if (added.length === 0) return;
      added.forEach(registerInlineAttachment);
      setAttachments((previous) => [...previous, ...added]);
      added.forEach((attachment) => editorRef.current?.insertImage(attachment));
    });
  };

  const removeAttachment = (id: string): void => {
    setAttachments((previous) =>
      previous.filter((attachment) => attachment.id !== id)
    );
  };

  // A native textarea cannot render CM6 image decorations. When returning from
  // fullscreen (or restoring a compact Mobile draft), remove only the visual
  // placement tokens while retaining every attachment byte. The legacy
  // no-token content path still sends those images before the prompt text.
  const demoteInlineImages = useCallback((): string => {
    const next = stripImageTokens(textRef.current);
    if (next === textRef.current) return next;
    textRef.current = next;
    initialText.current = next;
    setTextState(next);
    return next;
  }, []);

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

  const sendable = text.trim().length > 0 || attachments.length > 0;
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
    demoteInlineImages,
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
