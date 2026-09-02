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

/** Choose the immediate transcript presentation only when the prompt is likely
 *  to dispatch. Both transcript and queued presentation use the same durable
 *  IndexedDB mutation lane, so this predicate affects placement, not safety. */
export function shouldUseTranscriptDelivery(
  connected: boolean,
  dispatchable: boolean,
  queueEmpty: boolean,
): boolean {
  return connected && dispatchable && queueEmpty;
}
