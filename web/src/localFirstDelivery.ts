import { shouldUseTranscriptDelivery } from "./durableDelivery.ts";

/** Where a local-first send started. Drives the failed-row "return to X" home. */
export type DeliveryOrigin = "composer" | "draft" | "queue";

/** Parked home a failed send can return to. */
export type DeliveryHome = "draft" | "queue";

/** Visible chrome for a local row that has not been confirmed by the service. */
export type PendingSyncAppearance = "hidden" | "syncing" | "sending" | "failed";

export type DeliveryStatus = "pending" | "sending" | "failed";

export type DeliveryDestination = "transcript" | "queue";

export function homeForOrigin(origin: DeliveryOrigin): DeliveryHome {
  return origin === "queue" ? "queue" : "draft";
}

export function returnLabelForHome(home: DeliveryHome): string {
  return home === "queue" ? "Return to queue" : "Return to drafts";
}

/** First attempt: a closed socket is "waiting to resend", not a failure. */
export function firstDeliveryAttempt(sent: boolean): {
  readonly status: "pending";
  readonly armConfirmationTimeout: boolean;
} {
  return {
    status: "pending",
    armConfirmationTimeout: sent,
  };
}

/** An explicit Retry that still cannot leave this device is a network error. */
export function retryDeliveryAttempt(sent: boolean): {
  readonly status: DeliveryStatus;
  readonly armConfirmationTimeout: boolean;
} {
  if (sent) {
    return { status: "pending", armConfirmationTimeout: true };
  }
  return { status: "failed", armConfirmationTimeout: false };
}

/**
 * Unconfirmed row chrome.
 *
 * A connected `pending` row stays visually quiet so a fast LAN confirm never
 * flashes a loader. A disconnected `pending` row is waiting to reach the
 * service and must look like an upload. `sending` is an in-flight attempt that
 * already left this device. `failed` is a retry that still had no network, or
 * an ack timeout.
 */
export function pendingSyncAppearance(
  status: DeliveryStatus | undefined,
  connected: boolean,
): PendingSyncAppearance {
  if (status === "failed") return "failed";
  if (status === "sending") return "sending";
  if (status === "pending" && !connected) return "syncing";
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
