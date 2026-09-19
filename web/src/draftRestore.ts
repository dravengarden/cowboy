// Pure policy for restoring a composer draft whose attachment bytes live in
// IndexedDB while its text is mirrored in localStorage
// (docs/offline-first-sync.md, conflict 19).
import { type Attachment, dropOrphanImageTokens } from "./attachments";

export interface DraftMirror {
  readonly text: string;
  readonly attachments: readonly Attachment[];
  /** The mirror kept only text; the bytes were written to IndexedDB. */
  readonly attachmentsInDatabase?: boolean;
}

export interface StoredDraft {
  readonly text: string;
  readonly attachments: readonly Attachment[];
  readonly savedAt: number;
}

export interface RestoredDraft {
  readonly text: string;
  readonly attachments: Attachment[];
}

/**
 * Merge the synchronous mirror with the database record. The mirror's text is
 * the freshest (it is written on every keystroke flush); the record supplies
 * the bytes. A mirror that promised bytes the database no longer has drops the
 * orphaned inline tokens so nothing paints as a stray chip. Without a mirror
 * the record is the whole draft.
 */
export function mergeRestoredDraft(
  mirror: DraftMirror | undefined,
  record: StoredDraft | null,
): RestoredDraft | null {
  if (mirror === undefined) {
    if (record === null) return null;
    return { text: record.text, attachments: [...record.attachments] };
  }
  const attachments = record === null
    ? [...mirror.attachments]
    : mirror.attachments.length > 0
    ? [...mirror.attachments]
    : [...record.attachments];
  const ids = new Set(attachments.map((attachment) => attachment.id));
  const text = mirror.attachmentsInDatabase === true || record !== null
    ? dropOrphanImageTokens(mirror.text, ids)
    : mirror.text;
  if (text === mirror.text && attachments.length === mirror.attachments.length) {
    return null;
  }
  return { text, attachments };
}

/** Whether a record is worth reading back: only completed attachments persist. */
export function storableAttachments(attachments: readonly Attachment[]): Attachment[] {
  return attachments.filter((attachment) => attachment.pending !== true);
}
