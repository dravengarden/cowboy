export interface DurableDeliveryAttempt {
  readonly status: "pending";
  readonly armConfirmationTimeout: boolean;
}

/** A queue/draft mutation is already in the IndexedDB outbox before its first
 *  transport attempt. A closed socket therefore means "waiting to resend",
 *  not "failed"; only an attempt that actually left this device gets an ack
 *  timeout. */
export function durableDeliveryAttempt(sent: boolean): DurableDeliveryAttempt {
  return {
    status: "pending",
    armConfirmationTimeout: sent,
  };
}

/** Transcript optimism has no durable outbox. Use it only when the socket can
 *  carry an immediately-dispatchable prompt; every reconnect window falls back
 *  to the durable queue mutation path. */
export function shouldUseTranscriptDelivery(
  connected: boolean,
  dispatchable: boolean,
  queueEmpty: boolean,
): boolean {
  return connected && dispatchable && queueEmpty;
}
