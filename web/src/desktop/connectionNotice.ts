// The Desktop connection notice: what the strip above the composer says while
// the Cowboy server cannot be used (docs/offline-first-sync.md §Desktop). The
// status line segment stays the always-present indicator; this strip is where
// the eyes are when writing and sending, so a lasting outage is visible there.

import {
  relativeAge,
  type SyncPhase,
  type SyncStatus,
  syncStatusLabel,
} from "../syncStatus";

export interface ConnectionNotice {
  readonly tone: "warning" | "success";
  readonly title: string;
  /** What to check, or null when there is nothing for the user to do. */
  readonly hint: string | null;
  /** Retry countdown, last sync age and queued count, already joined. */
  readonly meta: string | null;
  readonly canRetry: boolean;
}

/** Phases that earn the strip. A short `connecting` blip stays in the status
 * line; sign-in and account fences already own a full decision surface. */
const NOTICE_PHASES: ReadonlySet<SyncPhase> = new Set([
  "offline",
  "unreachable",
  "degraded",
  "waiting",
]);

function hintFor(phase: SyncPhase): string | null {
  switch (phase) {
    case "offline":
      return "This device has no network. Check Wi-Fi or Ethernet.";
    case "unreachable":
      return "The network is up but the server is not answering. Check the VPN or the server.";
    case "degraded":
      return "Cowboy stopped answering; waiting for a heartbeat.";
    case "waiting":
      return "Another client holds this account's active seat.";
    default:
      return null;
  }
}

/**
 * @param presented the debounced phase from `presentedSyncPhase`, so a
 * reconnect blip never flashes the strip.
 */
export function connectionNotice(
  status: SyncStatus,
  presented: SyncPhase | "recovered" | null,
  now: number,
): ConnectionNotice | null {
  if (presented === "recovered") {
    return {
      tone: "success",
      title: "Reconnected",
      hint: null,
      meta: null,
      canRetry: false,
    };
  }
  if (
    presented === null || !NOTICE_PHASES.has(presented) ||
    presented !== status.phase
  ) {
    return null;
  }
  const title = status.phase === "unreachable"
    ? "Can't reach Cowboy server"
    : status.phase === "offline"
    ? "Offline"
    : syncStatusLabel(status, now) ?? "Reconnecting…";
  const meta: string[] = [];
  if (status.retryAt !== undefined && status.retryAt > now + 1_000) {
    meta.push(
      `Retrying in ${String(Math.ceil((status.retryAt - now) / 1000))} s`,
    );
  }
  const synced = relativeAge(status.lastLiveAt, now);
  if (synced !== null) meta.push(`last synced ${synced}`);
  const pending = status.outbox.pending;
  meta.push(
    pending > 0
      ? `${String(pending)} ${
        pending === 1 ? "message" : "messages"
      } will send automatically`
      : "messages you write are saved and send automatically",
  );
  const joined = meta.join(" · ");
  return {
    tone: "warning",
    title,
    hint: hintFor(status.phase),
    meta: joined.charAt(0).toUpperCase() + joined.slice(1),
    canRetry: true,
  };
}
