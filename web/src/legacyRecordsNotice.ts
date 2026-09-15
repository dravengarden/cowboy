/** Retained pre-dataset records are never adopted, resent or deleted, so their
 * set is stable for the life of the device profile. Warning about them on every
 * load is a notice the reader cannot act on twice; announce one set once and
 * leave Settings → Info as the durable review surface. */

const ANNOUNCED_KEY = "cowboy:legacy-records-announced";

export interface AnnouncementMemory {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** Stable, order-independent and bounded: the key list may hold thousands of
 * entries, and only a change of set justifies warning again. */
export function legacyRecordsFingerprint(keys: readonly string[]): string {
  let hash = 0x811c9dc5;
  for (const key of [...keys].sort()) {
    for (let i = 0; i < key.length; i += 1) {
      hash = Math.imul(hash ^ key.charCodeAt(i), 0x01000193) >>> 0;
    }
    hash = Math.imul(hash ^ 0x0a, 0x01000193) >>> 0;
  }
  return `${keys.length}:${hash.toString(16)}`;
}

function deviceMemory(): AnnouncementMemory | null {
  try {
    return globalThis.localStorage;
  } catch {
    return null;
  }
}

/** True when this exact retained set has not been announced on this device.
 * Records the announcement as a side effect, so a caller must warn when it
 * returns true. Pass `null` for a device without durable memory: warning again
 * is the safe direction. */
export function shouldAnnounceLegacyRecords(
  keys: readonly string[],
  memory: AnnouncementMemory | null = deviceMemory(),
): boolean {
  if (!keys.length) return false;
  const fingerprint = legacyRecordsFingerprint(keys);
  try {
    if (memory?.getItem(ANNOUNCED_KEY) === fingerprint) return false;
    memory?.setItem(ANNOUNCED_KEY, fingerprint);
  } catch {
    // A full or blocked store must not suppress the notice.
  }
  return true;
}
