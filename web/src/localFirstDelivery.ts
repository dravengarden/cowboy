import { stripImageTokens } from "./attachments.ts";
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

/**
 * What becomes of an unconfirmed send once its deadline ends.
 *
 * A transcript prompt whose transport timed out is parked in the session's
 * drafts instead of sitting in the transcript as a red row. Its fate is unknown,
 * the conversation may already have moved on (often from another device), and
 * resending it automatically could inject stale text into a later turn. A
 * draft keeps the content, syncs to every device, and leaves the decision to
 * the user without claiming an error.
 *
 * Everything else stays held: a Hub refusal carries a reason the user must see,
 * a stalled local write cannot be parked through the same wedged database, and
 * queue edits and moves have their own surface.
 */
export function unconfirmedSendDisposition(input: {
  readonly mutation: string;
  readonly phase: DeliveryStatus;
  readonly refused: boolean;
}): "draft" | "hold" {
  if (input.mutation !== "submitPrompt") return "hold";
  if (input.refused || input.phase === "committing") return "hold";
  return "draft";
}

/** Slack before a late timer callback counts as a frozen page. */
export const FROZEN_TIMER_SLACK_MS = 5_000;

/**
 * Whether an acknowledgement deadline that just fired is evidence of a lost
 * send. Time the page spent suspended, hidden, disconnected, or still
 * reconnecting is not: mobile browsers freeze timers in the background and run
 * them on resume before the socket or the visibility event catches up, which
 * would otherwise park a prompt the agent is already answering. Such a deadline
 * is re-armed instead of declaring the send undelivered.
 */
export function deliveryDeadlineDeferred(input: {
  readonly armedAt: number;
  readonly delayMs: number;
  readonly now: number;
  readonly visible: boolean;
  readonly connectedSince: number | null;
  readonly visibleSince: number;
  readonly settleMs: number;
}): boolean {
  if (!input.visible || input.connectedSince === null) return true;
  if (input.now - input.armedAt > input.delayMs + FROZEN_TIMER_SLACK_MS) return true;
  return input.now - input.connectedSince < input.settleMs ||
    input.now - input.visibleSince < input.settleMs;
}

/** A recovery draft whose original prompt is in the transcript after all is a
 * duplicate, unless the user has since edited it. Works without tab-local
 * parking state, so a reload or another device can retire it too. */
export function recoveryDraftMatchesEcho(draftText: string, echoText: string): boolean {
  const normalize = (text: string): string => stripImageTokens(text).replace(/\s/g, "");
  return normalize(draftText) === normalize(echoText);
}

const RECOVERY_DRAFT_PREFIX = "recovery-";

/** Deterministic id for the draft that preserves one parked send, so parking
 * the same send twice (a retry, a reload) cannot create a second draft. */
export function recoveryDraftCmid(sendId: string): string {
  return `${RECOVERY_DRAFT_PREFIX}${sendId}`;
}
/** The parked send a recovery draft preserves, or null for any other draft. */
export function recoveredSendId(draftCmid: string): string | null {
  return draftCmid.startsWith(RECOVERY_DRAFT_PREFIX) && draftCmid.length > RECOVERY_DRAFT_PREFIX.length
    ? draftCmid.slice(RECOVERY_DRAFT_PREFIX.length)
    : null;
}

/** A send that was parked and then echoed late did reach the agent. Its
 * draft is a duplicate unless the user has already edited it. */
export function lateEchoRetiresRecoveryDraft(
  parked: { readonly text: string; readonly attachments: number },
  draft: { readonly text: string; readonly attachments: readonly unknown[] },
): boolean {
  return draft.text === parked.text &&
    draft.attachments.length === parked.attachments;
}

export function destinationForPrompt(
  dispatchable: boolean,
  queueEmpty: boolean,
): DeliveryDestination {
  return shouldUseTranscriptDelivery(dispatchable, queueEmpty)
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
