import { useSyncExternalStore } from "react";

// Per-session "stick to bottom" (auto-scroll) state, shared between the
// Transcript — which owns the scroll engine and flips this on real user scroll —
// and the composer's sticky toggle, which displays + drives it. Keyed by
// session_id (like draftStore) so a detached scroll position in one session
// never shows on another's toggle; only the active session's Transcript +
// Composer are mounted, but keying also avoids a 1-frame stale flash on switch.
//
// `sticky` true = follow the latest message as content streams. `scrollNonce` is
// a monotonic counter the composer bumps to ask the Transcript to scroll to the
// bottom NOW (the "catch up + follow again" tap) — a nonce rather than a boolean
// so repeated taps each fire. Reactive via useSyncExternalStore, matching
// vimSetting / readingSettings / draftStore.

interface SessionSticky {
  sticky: boolean;
  scrollNonce: number;
}

// Shared default; safe to share since callers only read primitives off it.
const DEFAULT: SessionSticky = { sticky: true, scrollNonce: 0 };

const map = new Map<string, SessionSticky>();
const listeners = new Set<() => void>();

function get(sessionId: string): SessionSticky {
  return map.get(sessionId) ?? DEFAULT;
}

function emit(): void {
  for (const l of listeners) l();
}

function subscribe(onChange: () => void): () => void {
  listeners.add(onChange);
  return () => {
    listeners.delete(onChange);
  };
}

// Set sticky on/off for a session. No-op (no emit) when unchanged, so the
// Transcript's per-scroll-frame writes don't churn React.
export function setSticky(sessionId: string, sticky: boolean): void {
  const cur = get(sessionId);
  if (cur.sticky === sticky) return;
  map.set(sessionId, { sticky, scrollNonce: cur.scrollNonce });
  emit();
}

// Re-enable sticky AND ask the Transcript to scroll to the bottom now (the
// composer toggle's tap while detached).
export function requestStickToBottom(sessionId: string): void {
  const cur = get(sessionId);
  map.set(sessionId, { sticky: true, scrollNonce: cur.scrollNonce + 1 });
  emit();
}

// Reset to the default (sticky-on) — called when a session is (re)opened so the
// chat starts pinned to the latest message regardless of a prior detached state.
export function resetSticky(sessionId: string): void {
  const cur = get(sessionId);
  if (cur.sticky) return;
  map.set(sessionId, { sticky: true, scrollNonce: cur.scrollNonce });
  emit();
}

export function useSticky(sessionId: string): boolean {
  return useSyncExternalStore(
    subscribe,
    () => get(sessionId).sticky,
    () => true,
  );
}

export function useScrollNonce(sessionId: string): number {
  return useSyncExternalStore(
    subscribe,
    () => get(sessionId).scrollNonce,
    () => 0,
  );
}
