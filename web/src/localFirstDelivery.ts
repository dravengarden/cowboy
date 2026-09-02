import { shouldUseTranscriptDelivery } from "./durableDelivery.ts";

/** Where a local-first send started. Drives the failed-row "return to X" home. */
export type DeliveryOrigin = "composer" | "draft" | "queue";

/** Parked home a failed send can return to. */
export type DeliveryHome = "draft" | "queue";

/** Visible chrome for a local row that has not been confirmed by the service. */
export type PendingSyncAppearance = "hidden" | "saving" | "syncing" | "sending" | "failed";

export type DeliveryStatus = "committing" | "pending" | "sending" | "failed";

export type DeliveryDestination = "transcript" | "queue";

export function homeForOrigin(origin: DeliveryOrigin): DeliveryHome {
  return origin === "queue" ? "queue" : "draft";
}

export function returnLabelForHome(home: DeliveryHome): string {
  return home === "queue" ? "Return to queue" : "Return to drafts";
}

/** First attempt: a closed socket is "waiting to resend", not a failure. */
export function firstDeliveryAttempt(sent: boolean): {
  readonly status: "pending" | "sending";
  readonly armConfirmationTimeout: boolean;
} {
  return {
    status: sent ? "sending" : "pending",
    armConfirmationTimeout: sent,
  };
}

/** An explicit Retry that still cannot leave this device is a network error. */
export function retryDeliveryAttempt(sent: boolean): {
  readonly status: DeliveryStatus;
  readonly armConfirmationTimeout: boolean;
} {
  if (sent) {
    return { status: "sending", armConfirmationTimeout: true };
  }
  return { status: "failed", armConfirmationTimeout: false };
}

/**
 * Unconfirmed row chrome.
 *
 * `committing` is the local durability barrier, before transport is allowed to
 * see the mutation. `pending` has been committed but is waiting for a usable
 * connection. `sending` already left this device. None of these states is
 * visually quiet: user-authored content must acknowledge the tap immediately.
 */
export function pendingSyncAppearance(
  status: DeliveryStatus | undefined,
  connected: boolean,
): PendingSyncAppearance {
  if (status === "failed") return "failed";
  if (status === "committing") return "saving";
  if (!connected && (status === "pending" || status === "sending")) {
    return "syncing";
  }
  if (status === "sending") return "sending";
  if (status === "pending") return "syncing";
  return "hidden";
}

/** After an explicit send, show loading immediately instead of the 200ms quiet window. */
export function statusAfterExplicitSend(sent: boolean): DeliveryStatus {
  return sent ? "sending" : "pending";
}

export function destinationForPrompt(
  connected: boolean,
  dispatchable: boolean,
  queueEmpty: boolean,
): DeliveryDestination {
  return shouldUseTranscriptDelivery(connected, dispatchable, queueEmpty)
    ? "transcript"
    : "queue";
}

export function canReturnFromPendingRow(
  kind: "queued" | "draft",
  origin: DeliveryOrigin | undefined,
): boolean {
  if (kind === "queued") return true;
  return homeForOrigin(origin ?? "composer") === "queue";
}
