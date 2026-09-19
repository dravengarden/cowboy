// One derived connectivity/sync status for every product surface
// (docs/offline-first-sync.md §Architecture 4). The store feeds raw inputs;
// this module owns the phase derivation and the user-facing copy so Mobile and
// Desktop present the same truth.

export type SyncPhase =
  | "live"
  | "connecting"
  | "waiting"
  | "degraded"
  | "offline"
  | "auth_required"
  | "fenced";

export interface SyncOutboxSummary {
  /** Durable mutations that have not been confirmed by the Hub. */
  readonly pending: number;
  /** Rows the user must look at: timed out and held, or rejected. */
  readonly held: number;
  /** Sessions that own at least one pending or held row. */
  readonly sessions: readonly string[];
}

export interface SyncStatus {
  readonly phase: SyncPhase;
  /** When the current phase began (epoch ms). */
  readonly since: number;
  /** Next automatic reconnect attempt, when one is scheduled. */
  readonly retryAt?: number;
  /** Capacity queue position, 1-based, while `waiting`. */
  readonly position?: number;
  /** Last moment the Hub was live for this device, when known. */
  readonly lastLiveAt?: number;
  readonly outbox: SyncOutboxSummary;
  readonly updateReady: boolean;
}

export interface SyncStatusInput {
  /** Bootstrap complete and the socket admitted. */
  readonly connected: boolean;
  readonly socket: "none" | "connecting" | "open";
  readonly online: boolean;
  /** Consecutive reconnect attempts since the outage began. */
  readonly attempts: number;
  readonly retryAt?: number;
  readonly capacity?: "active" | "waiting" | "channel_limit" | "lost" | "unavailable";
  readonly position?: number;
  readonly pausedForAuth: boolean;
  readonly fenced: boolean;
  /** Milliseconds since the last frame on an open socket. */
  readonly silenceMs: number;
  readonly lastLiveAt?: number;
  readonly outbox: SyncOutboxSummary;
  readonly updateReady: boolean;
}

/** A healthy socket carries a heartbeat every 25 s; beyond this silence the
 * connection is presented as unstable before the 60 s watchdog replaces it. */
export const SYNC_DEGRADED_SILENCE_MS = 30_000;
/** Outage attempts before a blip is presented as being offline. Matches the
 * connection banner's historical threshold. */
export const SYNC_OFFLINE_ATTEMPTS = 2;
/** Mobile/Desktop presentation waits this long before showing a non-live
 * phase, so a foreground reconnect never flashes chrome. */
export const SYNC_PRESENTATION_DEBOUNCE_MS = 1_500;
/** How long the green "Synced" confirmation lingers after recovery. */
export const SYNC_RECOVERED_FLASH_MS = 2_000;

export function deriveSyncPhase(input: SyncStatusInput): SyncPhase {
  if (input.fenced) return "fenced";
  if (input.pausedForAuth) return "auth_required";
  if (input.connected && input.socket === "open") {
    return input.silenceMs > SYNC_DEGRADED_SILENCE_MS ? "degraded" : "live";
  }
  if (input.capacity === "waiting" || input.capacity === "channel_limit") {
    return "waiting";
  }
  if (!input.online || input.attempts >= SYNC_OFFLINE_ATTEMPTS) return "offline";
  return "connecting";
}

export function deriveSyncStatus(
  input: SyncStatusInput,
  previous: SyncStatus | undefined,
  now: number,
): SyncStatus {
  const phase = deriveSyncPhase(input);
  const since = previous !== undefined && previous.phase === phase ? previous.since : now;
  const status: SyncStatus = {
    phase,
    since,
    outbox: input.outbox,
    updateReady: input.updateReady,
    ...(input.retryAt !== undefined && phase === "connecting" ? { retryAt: input.retryAt } : {}),
    ...(input.position !== undefined && phase === "waiting" ? { position: input.position } : {}),
    ...(input.lastLiveAt !== undefined ? { lastLiveAt: input.lastLiveAt } : {}),
  };
  if (
    previous !== undefined &&
    previous.phase === status.phase &&
    previous.since === status.since &&
    previous.retryAt === status.retryAt &&
    previous.position === status.position &&
    previous.lastLiveAt === status.lastLiveAt &&
    previous.updateReady === status.updateReady &&
    previous.outbox.pending === status.outbox.pending &&
    previous.outbox.held === status.outbox.held &&
    previous.outbox.sessions.length === status.outbox.sessions.length &&
    previous.outbox.sessions.every((id, index) => id === status.outbox.sessions[index])
  ) {
    return previous;
  }
  return status;
}

export function ordinal(position: number): string {
  const rem100 = position % 100;
  if (rem100 >= 11 && rem100 <= 13) return `${String(position)}th`;
  const rem10 = position % 10;
  const suffix = rem10 === 1 ? "st" : rem10 === 2 ? "nd" : rem10 === 3 ? "rd" : "th";
  return `${String(position)}${suffix}`;
}

export function relativeAge(fromMs: number | undefined, now: number): string | null {
  if (fromMs === undefined) return null;
  const seconds = Math.max(0, Math.round((now - fromMs) / 1000));
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${String(minutes)} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return hours === 1 ? "1 hour ago" : `${String(hours)} hours ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "1 day ago" : `${String(days)} days ago`;
}

function countLabel(count: number, singular: string, plural: string): string {
  return `${String(count)} ${count === 1 ? singular : plural}`;
}

/** Compact pill / status-line label. `null` means nothing is shown. */
export function syncStatusLabel(status: SyncStatus, now: number): string | null {
  switch (status.phase) {
    case "live":
      return status.outbox.held > 0
        ? `${countLabel(status.outbox.held, "message needs", "messages need")} attention`
        : null;
    case "connecting": {
      if (status.retryAt !== undefined && status.retryAt > now + 1_500) {
        return `Reconnecting in ${String(Math.ceil((status.retryAt - now) / 1000))} s`;
      }
      return "Reconnecting…";
    }
    case "waiting":
      return status.position !== undefined
        ? `Waiting for a seat (${ordinal(status.position)})`
        : "Waiting for a seat";
    case "degraded":
      return "Connection unstable";
    case "offline":
      return status.outbox.pending > 0
        ? `Offline · ${countLabel(status.outbox.pending, "queued", "queued")}`
        : "Offline";
    case "auth_required":
      return "Sign in to sync";
    case "fenced":
      return "Reload required";
  }
}

/** Longer explanation for the detail sheet or tooltip. */
export function syncStatusDetail(status: SyncStatus, now: number): string {
  const synced = relativeAge(status.lastLiveAt, now);
  const queued = status.outbox.pending > 0
    ? ` ${countLabel(status.outbox.pending, "message", "messages")} will send automatically.`
    : "";
  switch (status.phase) {
    case "live":
      return status.outbox.held > 0
        ? "Some messages could not be confirmed. Retry, edit, or discard them."
        : "Connected to Cowboy.";
    case "connecting":
      return `Trying to reach Cowboy.${queued}`;
    case "waiting":
      return "Another client holds this account's active seat. Cowboy retries automatically.";
    case "degraded":
      return `Last heard from Cowboy ${relativeAge(now - 0, now) === "just now" ? "a moment ago" : "recently"}; waiting for a heartbeat.`;
    case "offline":
      return `${synced ? `Last synced ${synced}. ` : ""}Everything you write is saved on this device.${queued}`;
    case "auth_required":
      return status.outbox.pending > 0
        ? `${countLabel(status.outbox.pending, "queued message", "queued messages")} will send after you sign in.`
        : "Your sign-in needs to be renewed before Cowboy can sync.";
    case "fenced":
      return "This device was signed in as a different account. Reload to continue.";
  }
}

/** One session's held (timed out or refused) outbox rows, by mutation id. */
export interface HeldSession {
  readonly id: string;
  readonly ids: readonly string[];
}

/**
 * Held rows a status surface should point at. Rows of the opened session are
 * excluded: their own chrome already shows the failure with Retry, Return and
 * Discard. Rows the user has already dismissed stay quiet until a new one
 * appears, so a resolved row never re-raises the reminder for the rest.
 */
export function attentionCount(
  sessions: readonly HeldSession[],
  activeId: string | null | undefined,
  acknowledged: ReadonlySet<string>,
): number {
  let count = 0;
  for (const session of sessions) {
    if (session.id === activeId) continue;
    for (const id of session.ids) {
      if (!acknowledged.has(id)) count += 1;
    }
  }
  return count;
}

/** The same status with a different held count; identity is kept when equal. */
export function withHeld(status: SyncStatus, held: number): SyncStatus {
  if (status.outbox.held === held) return status;
  return { ...status, outbox: { ...status.outbox, held } };
}

/** Tone maps onto MUI palette keys; outages are calm, not alarms. */
export function syncStatusTone(
  phase: SyncPhase,
): "success" | "warning" | "info" | "error" {
  switch (phase) {
    case "live":
      return "success";
    case "connecting":
    case "waiting":
    case "degraded":
    case "offline":
      return "warning";
    case "auth_required":
      return "info";
    case "fenced":
      return "error";
  }
}

/**
 * Presentation gate for the pill/segment. A non-live phase is shown only after
 * it has lasted `SYNC_PRESENTATION_DEBOUNCE_MS`; a recovery flashes briefly.
 */
export function presentedSyncPhase(
  status: SyncStatus,
  previousShown: SyncPhase | null,
  recoveredAt: number | undefined,
  now: number,
): SyncPhase | "recovered" | null {
  if (status.phase === "live") {
    if (status.outbox.held > 0) return "live";
    if (recoveredAt !== undefined && now - recoveredAt < SYNC_RECOVERED_FLASH_MS) {
      return "recovered";
    }
    return null;
  }
  if (previousShown !== null && previousShown === status.phase) return status.phase;
  return now - status.since >= SYNC_PRESENTATION_DEBOUNCE_MS ? status.phase : null;
}
