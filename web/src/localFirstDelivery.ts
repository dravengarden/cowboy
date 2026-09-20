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

/** A frame that could not leave the tab is normally re-driven by the next
 * reconnect. That replay is not guaranteed to cover this row — a resend that
 * overlaps the row's own durable admission skips it — so the wait needs its own
 * bound. Long enough that an ordinary reconnect wins the race first. */
export const CONNECTED_PENDING_STALL_MS = 20_000;

/** A local outbox write that has not landed by here is blocked, not slow: an
 * IndexedDB request has no timeout of its own, and a `versionchange` blocked by
 * another tab never settles. */
export const COMMITTING_STALL_MS = 15_000;

/**
 * How long a locally-committed row may rest in each unconfirmed phase before
 * the user is handed an escape, or `null` when some other owner already ends
 * the phase.
 *
 * Every phase needs exactly one owner. `sending` has always had the
 * acknowledgement timeout; `pending` and `committing` had nothing, so a lost
 * transport or durability event parked a row on "Waiting for Cowboy…" or
 * "Saving…" indefinitely — no failure chrome, no Retry. Offline is the one
 * legitimate rest: `pending` with the socket down IS the offline-first
 * contract, and failing it would call a healthy queued prompt broken
 * (docs/offline-first-sync.md: "a timeout is not evidence of failure when the
 * socket is down"). Keep this total over `DeliveryStatus` so a new phase cannot
 * be added without naming its owner.
 */
export function deliveryStallMs(
  status: DeliveryStatus | undefined,
  connected: boolean,
): number | null {
  switch (status) {
    case "pending":
      return connected ? CONNECTED_PENDING_STALL_MS : null;
    case "committing":
      return COMMITTING_STALL_MS;
    // Owned by the acknowledgement timeout armed when the frame left the tab.
    case "sending":
      return null;
    // Terminal: already carries Retry / Return / Discard.
    case "failed":
      return null;
    case undefined:
      return null;
  }
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
