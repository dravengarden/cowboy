import {
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  type Attachment,
  filesToAttachments,
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
  addFiles: (files: File[]) => void;
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
