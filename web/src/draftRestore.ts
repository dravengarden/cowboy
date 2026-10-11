// Pure policy for restoring a composer draft whose attachment bytes live in
// IndexedDB while its text is mirrored in localStorage
// (docs/offline-first-sync.md, conflict 19).
import {
  type Attachment,
  dropOrphanImageTokens,
  imageTokensInText,
} from "./attachments";

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
 * Merge the live draft with the database record that was read successfully.
 * The live text is the freshest (it is mirrored on every keystroke flush and
 * may have been edited since hydration); the record supplies the bytes.
 *
 * `awaiting` says the mirror promised bytes in the database. Only then may a
 * missing record drop the orphaned inline tokens, and only then are record
 * attachments adopted: an image returns when its token is still in the text
 * (or nothing was typed since hydration), a tokenless file always returns.
 * A mirror that never promised bytes is complete and ignores a stale record.
 * Without a live draft the record is the whole draft.
 *
 * Callers must not pass a failed read as `null`: an unreadable database says
 * nothing about the bytes, and treating it as absence deletes the images.
 */
export function mergeRestoredDraft(
  live: DraftMirror | undefined,
  record: StoredDraft | null,
  hydratedText?: string,
): RestoredDraft | null {
  if (live === undefined) {
    if (record === null) return null;
    return { text: record.text, attachments: [...record.attachments] };
  }
  if (live.attachmentsInDatabase !== true) return null;
  const attachments = [...live.attachments];
  const known = new Set(attachments.map((attachment) => attachment.id));
  const unedited = hydratedText === undefined || hydratedText === live.text;
  const tokens = new Set(imageTokensInText(live.text).map((token) => token.id));
  for (const attachment of record?.attachments ?? []) {
    if (known.has(attachment.id)) continue;
    if (attachment.isImage && !unedited && !tokens.has(attachment.id)) continue;
    attachments.push(attachment);
    known.add(attachment.id);
  }
  const text = dropOrphanImageTokens(live.text, known);
  if (text === live.text && attachments.length === live.attachments.length) {
    return null;
  }
  return { text, attachments };
}

/** Whether a record is worth reading back: only completed attachments persist. */
export function storableAttachments(attachments: readonly Attachment[]): Attachment[] {
  return attachments.filter((attachment) => attachment.pending !== true);
}
