export interface DurableDeliveryAttempt {
  readonly status: "pending" | "sending";
  readonly armConfirmationTimeout: boolean;
}

/** A queue/draft mutation is already in the IndexedDB outbox before its first
 *  transport attempt. A closed socket therefore means "waiting to resend",
 *  not "failed"; only an attempt that actually left this device gets an ack
 *  timeout. */
export function durableDeliveryAttempt(sent: boolean): DurableDeliveryAttempt {
  return {
    status: sent ? "sending" : "pending",
    armConfirmationTimeout: sent,
  };
}

/** Persist source acknowledgements before a dependent move can consume that
 * source on the server. Otherwise a crash can replay the already-consumed
 * creation from disk. Recheck ownership after the asynchronous write: discard
 * or an authoritative acknowledgement may have retired the move meanwhile. */
export async function sendAfterDurableSnapshot(
  store: {
    flush: () => Promise<void>;
    pending: () => readonly { id: string }[];
  },
  mutationId: string,
  send: () => void,
): Promise<void> {
  await store.flush();
  if (store.pending().some((mutation) => mutation.id === mutationId)) send();
}

/** A failed discard restores the outbox mutation. Restore a retryable delivery
 * phase too: an overlapping send barrier may have observed the temporary
 * removal and deliberately skipped transport. Never silently send after the
 * user tried to cancel; let the restored row offer an explicit retry. */
export async function discardDurableDelivery(
  store: {
    confirmDurably: (ids: readonly string[]) => Promise<void>;
    pending: () => readonly { id: string }[];
  },
  mutationId: string,
  onRollback: () => void,
): Promise<void> {
  try {
    await store.confirmDurably([mutationId]);
  } catch (error) {
    if (store.pending().some((mutation) => mutation.id === mutationId)) onRollback();
    throw error;
  }
}

/** Choose the immediate transcript presentation only when the prompt is likely
 *  to dispatch. Both transcript and queued presentation use the same durable
 *  IndexedDB mutation lane, so this predicate affects placement, not safety.
 *
 *  Connectivity is deliberately not an input. An idle session dispatches the
 *  outbox resend as soon as the socket returns; parking an offline prompt in the
 *  queue rail made it vanish from the conversation, then jump back once the
 *  echo crossed a slow link. The bubble stays put and shows it is unsent. */
export function shouldUseTranscriptDelivery(
  dispatchable: boolean,
  queueEmpty: boolean,
): boolean {
  return dispatchable && queueEmpty;
}
