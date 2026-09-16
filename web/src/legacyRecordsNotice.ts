/** Retained pre-dataset records are never adopted, resent or deleted, and
 * Settings → Info lists every one of them for as long as they exist. The toast
 * is therefore a POINTER at a durable surface, not the surface — so it is worth
 * exactly one appearance per device.
 *
 * It used to gate on a fingerprint of the retained key set, which meant any
 * drift in that set re-announced. On an iPad PWA — which reloads whenever iOS
 * evicts the web view — that produced roughly twenty identical warnings in a
 * morning, none of which told the reader anything the previous one had not.
 * Gating is now on the ANNOUNCEMENT itself: once told, never told again, until
 * the retained set actually empties and a genuinely new one appears. */

const ANNOUNCED_KEY = "cowboy:legacy-records-announced";
const ANNOUNCED_VALUE = "1";

export interface AnnouncementMemory {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export type LegacyAnnouncementReason =
  /** Nothing retained: nothing to say, and a later set may speak once. */
  | "empty"
  /** First time this device has been told. Warn. */
  | "fresh"
  /** Already told. Settings → Info still has the records. */
  | "already-announced"
  /** The device cannot remember having been told (storage blocked, full, or
   * absent). Staying silent is deliberate: see below. */
  | "unrecordable";

export interface LegacyAnnouncement {
  readonly announce: boolean;
  readonly reason: LegacyAnnouncementReason;
  readonly count: number;
}

function deviceMemory(): AnnouncementMemory | null {
  try {
    return globalThis.localStorage;
  } catch {
    return null;
  }
}

/**
 * Decide whether to warn, and record the decision.
 *
 * The unrecordable case is the one worth stating plainly: when the marker
 * cannot be written and read back, this returns `announce: false`. A warning
 * the device cannot remember making is a warning it will make on EVERY load —
 * which is precisely the failure being fixed — and the same records stay listed
 * in Settings → Info either way. A notice that cannot be silenced is worse than
 * one that was never louder than the durable surface behind it.
 */
export function legacyRecordsAnnouncement(
  keys: readonly string[],
  memory: AnnouncementMemory | null = deviceMemory(),
): LegacyAnnouncement {
  const count = keys.length;
  if (count === 0) {
    // Forget, so a genuinely new retained set can announce once later.
    try {
      memory?.removeItem(ANNOUNCED_KEY);
    } catch {
      // Nothing to clean up is not a failure worth reporting.
    }
    return { announce: false, reason: "empty", count };
  }
  if (!memory) return { announce: false, reason: "unrecordable", count };
  try {
    if (memory.getItem(ANNOUNCED_KEY) === ANNOUNCED_VALUE) {
      return { announce: false, reason: "already-announced", count };
    }
    memory.setItem(ANNOUNCED_KEY, ANNOUNCED_VALUE);
    // Read back rather than trust the write: iOS drops localStorage writes when
    // the origin's quota is exhausted, and a silent failure here is what turns
    // one notice into one per reload.
    if (memory.getItem(ANNOUNCED_KEY) !== ANNOUNCED_VALUE) {
      return { announce: false, reason: "unrecordable", count };
    }
  } catch {
    return { announce: false, reason: "unrecordable", count };
  }
  return { announce: true, reason: "fresh", count };
}
