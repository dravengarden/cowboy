/** Conversation Clear is a page transition, not a hard cut.
 *  Confirm starts the exit; the transcript keeps a snapshot until that
 *  motion finishes, then the empty canvas enters. */
export const conversationClearExitMs = 320;
export const conversationClearEnterMs = 280;

type ConversationClearListener = (sessionId: string) => void;

const listeners = new Set<ConversationClearListener>();

export function beginConversationClear(sessionId: string): void {
  for (const listener of listeners) listener(sessionId);
}

export function subscribeConversationClear(
  listener: ConversationClearListener,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function prefersReducedConversationMotion(): boolean {
  return globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").matches ===
    true;
}
