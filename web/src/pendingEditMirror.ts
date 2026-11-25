// Stateless localStorage mirror codec and key helpers for pending row edits.

import { type Attachment, dropOrphanImageTokens } from "./attachments";
import { newUuid } from "./uuid";

export type PendingEditKind = "queued" | "draft";

export interface PendingEditRecord {
  sessionId: string;
  kind: PendingEditKind;
  id: string;
  text: string;
  attachments: Attachment[];
  /** Content the server row had when this transaction began. Retained for
   * future conflict presentation; recovery never silently discards on mismatch. */
  baseFingerprint: string;
  /** Stable idempotency key used only if the edited server row disappears while
   * this client is offline and the buffer must be recovered as a parked draft. */
  recoveryCmid: string;
  updatedAt: number;
}

export interface PendingEditTarget {
  id: string;
  text: string;
  attachments: Attachment[];
}

export interface RestoredPendingEdit {
  readonly text: string;
  readonly attachments: Attachment[];
}

export interface HydratedPendingEditMirror {
  readonly key: string;
  readonly record: PendingEditRecord;
  readonly promised: boolean;
}

export const PENDING_EDIT_KEY_PREFIX = "cowboy:pending-edit:";

export function pendingEditRecordKey(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
): string {
  return JSON.stringify([sessionId, kind, id]);
}

export function pendingEditSessionOfKey(key: string): string | undefined {
  try {
    const parsed: unknown = JSON.parse(key);
    return Array.isArray(parsed) && typeof parsed[0] === "string" ? parsed[0] : undefined;
  } catch {
    return undefined;
  }
}

export function pendingEditStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

/** Compare only server-delivered content. Preview URLs and local presentation
 * metadata are deliberately excluded so reload-created object URLs do not make
 * identical edits look different. */
export function pendingEditFingerprint(
  text: string,
  attachments: readonly Attachment[],
): string {
  return JSON.stringify([text, attachments.map((attachment) => attachment.block)]);
}

export function parsePendingEditRecord(
  value: unknown,
  keepTokens = false,
): PendingEditRecord | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<PendingEditRecord>;
  if (
    typeof candidate.sessionId !== "string" ||
    (candidate.kind !== "queued" && candidate.kind !== "draft") ||
    typeof candidate.id !== "string" ||
    typeof candidate.text !== "string" ||
    !Array.isArray(candidate.attachments) ||
    typeof candidate.baseFingerprint !== "string" ||
    typeof candidate.updatedAt !== "number"
  ) return null;
  const attachments = candidate.attachments as Attachment[];
  const ids = new Set(attachments.map((attachment) => attachment.id));
  return {
    sessionId: candidate.sessionId,
    kind: candidate.kind,
    id: candidate.id,
    text: keepTokens ? candidate.text : dropOrphanImageTokens(candidate.text, ids),
    attachments,
    baseFingerprint: candidate.baseFingerprint,
    recoveryCmid: typeof candidate.recoveryCmid === "string"
      ? candidate.recoveryCmid
      : `pending-edit-${newUuid()}`,
    updatedAt: candidate.updatedAt,
  };
}

export function readHydratedPendingEditMirrors(): HydratedPendingEditMirror[] {
  const local = pendingEditStorage();
  if (local === null) return [];
  const entries: HydratedPendingEditMirror[] = [];
  try {
    for (let index = 0; index < local.length; index += 1) {
      const key = local.key(index);
      if (!key?.startsWith(PENDING_EDIT_KEY_PREFIX)) continue;
      try {
        const raw = local.getItem(key);
        const stored: unknown = raw === null ? null : JSON.parse(raw);
        // The bytes live in IndexedDB; keep the tokens so the restore can put
        // the images back exactly where they were.
        const promised = stored !== null && typeof stored === "object" &&
          (stored as { attachmentsInDatabase?: unknown }).attachmentsInDatabase === true &&
          Array.isArray((stored as { attachments?: unknown }).attachments) &&
          (stored as { attachments: unknown[] }).attachments.length === 0;
        const parsed = parsePendingEditRecord(stored, promised);
        if (parsed !== null) {
          entries.push({
            key: pendingEditRecordKey(parsed.sessionId, parsed.kind, parsed.id),
            record: parsed,
            promised,
          });
        }
      } catch {
        // Skip one corrupt entry without hiding later recovery records.
      }
    }
  } catch {
    // An unavailable localStorage must not prevent the app shell.
  }
  return entries;
}

export function writeLegacyPendingEditMirror(
  key: string,
  record: PendingEditRecord,
  strip = true,
): "full" | "stripped" | "failed" {
  const local = pendingEditStorage();
  if (local === null) return "failed";
  const storageKey = PENDING_EDIT_KEY_PREFIX + key;
  try {
    local.setItem(storageKey, JSON.stringify(record));
    return "full";
  } catch {
    if (!strip) return "failed";
    // Large embedded images can exceed the browser quota. Keep the irreplaceable
    // text and strip now-orphaned inline-image tokens; files can be reattached.
    try {
      local.setItem(
        storageKey,
        JSON.stringify({
          ...record,
          text: dropOrphanImageTokens(record.text, new Set()),
          attachments: [],
        }),
      );
      return "stripped";
    } catch {
      // The mounted editor and in-memory mirror remain the last-resort source.
      return "failed";
    }
  }
}

export function removePendingEditMirror(key: string): void {
  try {
    pendingEditStorage()?.removeItem(PENDING_EDIT_KEY_PREFIX + key);
  } catch {
    // The in-memory delete already succeeded.
  }
}

export function removeSessionPendingEditMirrors(sessionId: string): void {
  const local = pendingEditStorage();
  if (local === null) return;
  try {
    const stale: string[] = [];
    for (let index = 0; index < local.length; index += 1) {
      const storageKey = local.key(index);
      if (!storageKey?.startsWith(PENDING_EDIT_KEY_PREFIX)) continue;
      if (pendingEditSessionOfKey(storageKey.slice(PENDING_EDIT_KEY_PREFIX.length)) === sessionId) {
        stale.push(storageKey);
      }
    }
    for (const storageKey of stale) local.removeItem(storageKey);
  } catch {
    // Tolerant — in-memory cleanup already succeeded.
  }
}

export function mirrorNewerThan(key: string, updatedAt: number): boolean {
  try {
    const raw = pendingEditStorage()?.getItem(PENDING_EDIT_KEY_PREFIX + key);
    if (raw === null || raw === undefined) return false;
    const mirror: unknown = JSON.parse(raw);
    const stamp = (mirror as { updatedAt?: unknown } | null)?.updatedAt;
    return typeof stamp === "number" && stamp > updatedAt;
  } catch {
    return false;
  }
}
