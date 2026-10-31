// Opening a linked session lands where the link was made, not wherever that
// session was last read. One-shot: a later plain open keeps its own position.
import type { SessionMeta } from "./protocol";
import { requestPickSession } from "./sessionPickRequest";

export const SESSION_REVEAL_EVENT = "cowboy:session-reveal";
const pending = new Map<string, number>();

/** Open `id` at its history as of the exclusive boundary `before`. */
export function requestSessionReveal(id: string, session: SessionMeta | undefined, before: number): void {
  pending.set(id, before);
  requestPickSession(id, session);
  globalThis.dispatchEvent(new CustomEvent(SESSION_REVEAL_EVENT, { detail: id }));
}

export function takeSessionReveal(id: string): number | undefined {
  const before = pending.get(id);
  pending.delete(id);
  return before;
}

/** The newest row the boundary still includes. Rows are keyed by their first
 *  sequence number; other keys are optimistic and never history. */
export function lastRowBefore(keys: readonly string[], before: number): string | undefined {
  let found: string | undefined;
  let newest = -1;
  for (const key of keys) {
    const seq = /^\d+$/.test(key) ? Number(key) : NaN;
    if (seq < before && seq > newest) {
      newest = seq;
      found = key;
    }
  }
  return found;
}
