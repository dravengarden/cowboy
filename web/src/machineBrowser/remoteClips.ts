/** Text the remote browser copied, held for the open panel so the owner can
 * bring it to this device. Xvnc forwards only explicit copies; each one
 * becomes a clip and is offered once (the copy prompt). Clips live in the
 * panel's memory only: they survive viewer reconnects, hibernation and
 * reopen, and are dropped when the panel closes or binds another account
 * or browser. Nothing is persisted or sent anywhere.
 * See docs/machine-browser.md, "Remote clipboard". */

export interface RemoteClip {
  /** Unique within one panel's history. */
  id: number;
  text: string;
  /** When this device received it (ms since the epoch). */
  at: number;
}

export interface RemoteClips {
  /** Newest first. */
  items: readonly RemoteClip[];
  /** The clip the copy prompt offers, until it is taken or dismissed. */
  offer: number | undefined;
  next: number;
}

export const NO_CLIPS: RemoteClips = { items: [], offer: undefined, next: 1 };

export const MAX_CLIPS = 10;
/** All held clips together; the newest clip is always kept whole. */
export const MAX_CLIP_CHARS = 1_000_000;

/** A copy arrived from the remote browser. The same text as the newest clip
 * (the display server re-announcing its clipboard on reconnect) changes
 * nothing; an older duplicate moves to the top as a fresh copy. */
export function addClip(clips: RemoteClips, text: string, at: number): RemoteClips {
  if (!text || clips.items[0]?.text === text) return clips;
  const clip: RemoteClip = { id: clips.next, text, at };
  const items: RemoteClip[] = [clip];
  let total = text.length;
  for (const item of clips.items) {
    if (item.text === text) continue;
    if (items.length >= MAX_CLIPS || total + item.text.length > MAX_CLIP_CHARS) break;
    items.push(item);
    total += item.text.length;
  }
  return { items, offer: clip.id, next: clips.next + 1 };
}

/** The prompt was answered (copied, opened in the sheet or dismissed). */
export function settleOffer(clips: RemoteClips, id: number): RemoteClips {
  return clips.offer === id ? { ...clips, offer: undefined } : clips;
}

export function offeredClip(clips: RemoteClips): RemoteClip | undefined {
  return clips.offer === undefined
    ? undefined
    : clips.items.find((clip) => clip.id === clips.offer);
}

/** A one-line preview: whitespace collapsed, cut at `max` characters
 * (code points, so an emoji is never split). */
export function clipPreview(text: string, max = 48): string {
  const flat = text.replace(/\s+/gu, " ").trim();
  const chars = [...flat];
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : flat;
}

export function clipLength(text: string): number {
  return [...text].length;
}
