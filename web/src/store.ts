// Single WebSocket store shared by the whole app. cowboy is the source of
// truth; this store just accumulates what it pushes. Exposed via
// useSyncExternalStore so any component re-renders on change.
//
// All clients are equal subscribers: on connect the daemon sends a lightweight
// session index and live tail. Only the focused session hydrates its recent log
// over HTTP. We dedup by (session_id, seq), so HTTP/live overlap is harmless.

import { useCallback, useRef, useSyncExternalStore } from "react";
import { createConnectionStore } from "@cowboy/app-shell";
import {
  type ArgsOf,
  type ClientSnapshot,
  type Mutators,
  replicatedStore,
  type ReplicatedStore,
  snapshotPatch,
} from "@cowboy/state-sync";
import { idbListKeys, idbPersistence } from "@cowboy/state-sync-idb";
import { type Attachment, blocksToAttachments, buildContentBlocks } from "./attachments";
import {
  isAppleTouchWebView,
  shouldReconnectOnForeground,
  shouldStartImmediateReconnect,
} from "./connectionRecovery.ts";
import {
  durableDeliveryAttempt,
  shouldUseTranscriptDelivery,
} from "./durableDelivery.ts";
import {
  type DeliveryOrigin,
  destinationForPrompt,
  homeForOrigin,
  retryDeliveryAttempt,
  statusAfterExplicitSend,
} from "./localFirstDelivery.ts";
import {
  emptyQueueValue,
  QUEUE_TRANSITION_MUTATORS,
  queueMutators,
  type QueueValue,
  settledTransitionIds,
} from "./queueMutators.ts";
import { shouldApplyHydratedConfigOptions } from "./configOptionsHydration";
import { pruneDrafts } from "./draftStore";
import {
  claimOrphanedPendingEdits,
  finishOrphanedPendingEdit,
  prunePendingEdits,
} from "./pendingEditStore";
import { revealPendingArrival } from "./pendingPanelState";
import { linkTimeline } from "./derive";
import { beginConversationClear } from "./conversationClearance";
import { resetExploreAfterContextClear } from "./explore/exploreStore";
import { notifyHaptic } from "./haptic";
import { reportClientLog, reportClientMetric } from "./observability";
import { newUuid } from "./uuid";
import { fireAlert, vibrateAlertOn } from "./turnNotify";
import {
  acceptsMachineSnapshot,
  projectMachineOccupancy,
} from "./machineState";
import {
  type ConfigOption,
  type ContentBlock,
  type Delivery,
  type DraftSchedule,
  type Envelope,
  type Inbound,
  isPureTerminalOutputDelta,
  type MachineSummary,
  type Outbound,
  type SessionBootstrapResponse,
  type SessionMeta,
  type WireQueued,
} from "./protocol";
import { mergeCanonicalTimeline } from "./canonicalTimeline";
import { retainedEventCountForRows, retainTimelineState } from "./timelineRetention";
import { transcriptPresentationIntervalMs } from "./transcriptRenderPacing";
import {
  retainTranscriptSessionCache,
  touchTranscriptSessionCache,
} from "./transcriptSessionCache";

/// One notification slot — the App's snackbar shows the latest. We monotonically
/// bump `seq` even on repeat messages so the UI can re-trigger the open
/// animation for the same text (e.g. user sets mode twice fast).
export interface ErrorNotice {
  seq: number;
  sessionId?: string;
  message: string;
  /** Snackbar severity — defaults to "error" when absent. */
  severity?: "error" | "warning";
}

/// Top-of-app connection / version banner — the app-shell component instance
/// (see @cowboy/app-shell/connection-banner.tsx). Cowboy keeps its
/// canonical behavior: red "down" past the threshold, green "reconnected" flash,
/// blue "update" that counts 3→0 and clears caches + reloads on its own. The
/// store reports socket open/close into `conn` and reads back the backoff delay;
/// the banner state lives entirely in `conn` (App renders <ConnectionBanner>).
/// `versionUrl: "/version"` is cowboy's build-id endpoint.
export const conn = createConnectionStore({ versionUrl: "/version" });

/// One staged message — a queued prompt or a draft. SERVER-AUTHORITATIVE: the
/// daemon owns the per-session queue/drafts and the drain (next-on-turn-end), so
/// every connected terminal renders the same list (these used to be client-local
/// localStorage, which never synced across devices). The store receives the
/// canonical lists via the `queues` broadcast and mutates them by sending
/// commands; it never drains or serializes locally. `id` is assigned by the
/// daemon; `attachments` are reconstructed from the wire content blocks for
/// display / re-edit.
export interface QueuedMessage {
  id: string;
  text: string;
  /** Staged image / file attachments (reconstructed from the message's ACP
   *  content blocks). Empty for a plain text prompt. */
  attachments: Attachment[];
  /** Client message id. On a server row it's echoed back so the originating
   *  client can match its optimistic copy by id (never text). On a LOCAL
   *  optimistic row it's the id we minted. */
  cmid?: string;
  /** Set ONLY on a local optimistic row awaiting/failed daemon confirmation:
   *  `pending` (just sent, no shimmer yet — see SHIMMER_DELAY_MS), `sending`
   *  (still unconfirmed past the delay → gradient shimmer), `failed` (WS down /
   *  timed out → red + retry). A confirmed server row carries no status. */
  status?: "pending" | "sending" | "failed";
  /** Where this local row came from, so a failed send can return there. */
  origin?: DeliveryOrigin;
  /** Present only on a DRAFT with a future fire time — the server auto-activates
   *  it then. Drives the draft-row clock chip. Absent on plain drafts. */
  schedule?: DraftSchedule;
}

export interface State {
  connected: boolean;
  sessions: SessionMeta[];
  // Durable Machine registry projected by the Controller over the same product
  // WebSocket. Retain it across disconnects: transport loss is not evidence
  // that every enrolled Machine disappeared.
  machines: readonly MachineSummary[];
  machinesLoaded: boolean;
  machinesRevision: number;
  // session_id → seq-ordered, deduped event log
  timelines: Map<string, Envelope[]>;
  // session_ids whose history `snapshot` has arrived. Distinguishes "history
  // still loading" from "loaded, genuinely empty", so the transcript shows a
  // loading skeleton only during the initial fetch — not for an empty session.
  hydrated: Set<string>;
  // session_id → history pagination state. `reachedStart` = the loaded window
  // already includes the very first event; `loadingOlder` = a page fetch is in
  // flight (one at a time); `nextPage` = the next (older) seq-aligned page index
  // to fetch, DECREMENTED each load. Tracked (not recomputed from the oldest
  // seq) so a seq gap at a page boundary can't stall progress.
  pagination: Map<string, { reachedStart: boolean; loadingOlder: boolean; beforeSeq: number | null }>;
  // True once the first "sessions" list has arrived. Lets the UI tell "no
  // sessions yet (still loading)" from "loaded — the persisted focus is gone".
  sessionsLoaded: boolean;
  // session_id → agent-advertised configOptions array (mode/model/effort)
  configOptions: Map<string, ConfigOption[]>;
  // session_id → ordered prompts waiting for the current turn to finish.
  // Server-authoritative (the `queues` broadcast); the daemon drains them.
  queues: Map<string, QueuedMessage[]>;
  // session_id → parked DRAFT messages: composed but not committed to send.
  // Server-authoritative + persisted in postgres, so they sync across every
  // terminal and survive a daemon restart.
  drafts: Map<string, QueuedMessage[]>;
  // session_id → LOCAL optimistic CHAT bubbles (submit-when-idle → transcript)
  // awaiting the daemon's user-echo. Purely client-side, NOT synced/persisted;
  // reconciled away when their `cmid` lands (the user-echo Envelope, or — for a
  // wrong busy/idle guess — the queue `sync_patch`). Queue + drafts optimism now
  // lives in the per-session queue sync clients (see `qClient`), not here.
  optimisticMessages: Map<string, QueuedMessage[]>;
  lastError?: ErrorNotice;
  // session_id → title from the "title" state-sync channel. Source
  // of title truth on the client: overlaid onto SessionMeta.title by
  // `deriveSessions`, so an optimistic rename shows instantly and every terminal
  // converges on the arbiter's `sync_patch`. Ids absent here fall back to the
  // broadcast SessionMeta.title.
  titleOverrides: Record<string, string>;
  // Mobile-only code-review workspace state. The daemon persists and syncs it
  // across Mobile clients; Desktop UI never reads or writes this field.
  mobileReviewStates: Record<string, MobileReviewState>;
}

let errorSeq = 0;
let state: State = {
  connected: false,
  sessions: [],
  machines: [],
  machinesLoaded: false,
  machinesRevision: 0,
  timelines: new Map(),
  hydrated: new Set(),
  pagination: new Map(),
  sessionsLoaded: false,
  configOptions: new Map(),
  // Both populated from the server's `queues` broadcast (on connect + on every
  // change). Start empty; never written locally except by that broadcast.
  queues: new Map(),
  drafts: new Map(),
  optimisticMessages: new Map(),
  titleOverrides: {},
  mobileReviewStates: {},
};
// React reads only this published snapshot. `state` above remains canonical and
// can advance at websocket speed; notification pacing and gesture holds publish
// it atomically here so an unrelated local React render cannot accidentally
// pierce a frozen workspace by calling a selector between notifications.
let presentedState = state;
const listeners = new Set<() => void>();
let socket: WebSocket | undefined;
// Set by `cowboy:product-sign-out`. Prevents onclose / online / foreground
// from opening another product socket after logout.
let productSessionAbandoned = false;
// The session the user currently has open. Remembered so every (re)connect can
// re-assert it to the daemon (revive-on-open), recovering the agent after a
// daemon restart we reconnected across. See openSession + connect's onopen.
let openedSessionId: string | undefined;
interface SessionHydration {
  promise: Promise<void>;
  controller: AbortController;
}
const sessionHydrations = new Map<string, SessionHydration>();
const sessionHydrationRetryTimers = new Map<
  string,
  ReturnType<typeof setTimeout>
>();
const SESSION_HYDRATION_RETRY_DELAYS_MS = [750, 2_000] as const;
// Each live config snapshot advances this revision. HTTP bootstrap is allowed
// to seed the map only if no newer WebSocket snapshot arrived while it was in
// flight; otherwise its stale response can visibly undo a just-selected preset.
const configOptionsRevisions = new Map<string, number>();
// Transcript payloads are the dominant long-lived browser allocation (tool
// results and inline images can be megabytes). Keep only a small MRU working
// set; session metadata, queues, drafts, and persisted history remain intact.
let transcriptSessionCache: string[] = [];

function transcriptIsCached(sessionId: string): boolean {
  return transcriptSessionCache.includes(sessionId);
}

function evictTranscriptSessions(sessionIds: readonly string[]): void {
  if (sessionIds.length === 0) return;
  const evicted = new Set(sessionIds);
  for (const sessionId of evicted) {
    sessionHydrations.get(sessionId)?.controller.abort();
    sessionHydrations.delete(sessionId);
    const retry = sessionHydrationRetryTimers.get(sessionId);
    if (retry !== undefined) clearTimeout(retry);
    sessionHydrationRetryTimers.delete(sessionId);
    completeQuestionPages.delete(sessionId);
    transcriptEpoch.set(sessionId, (transcriptEpoch.get(sessionId) ?? 0) + 1);
  }

  const timelines = new Map(state.timelines);
  const hydrated = new Set(state.hydrated);
  const pagination = new Map(state.pagination);
  let changed = false;
  for (const sessionId of evicted) {
    changed = timelines.delete(sessionId) || changed;
    changed = hydrated.delete(sessionId) || changed;
    changed = pagination.delete(sessionId) || changed;
  }
  if (changed) {
    setState({ ...state, timelines, hydrated, pagination });
  }
}

function touchTranscriptSession(sessionId: string): void {
  const update = touchTranscriptSessionCache(transcriptSessionCache, sessionId);
  transcriptSessionCache = update.order;
  evictTranscriptSessions(update.evicted);
}

function retainTranscriptSessions(valid: ReadonlySet<string>): void {
  const update = retainTranscriptSessionCache(transcriptSessionCache, valid);
  transcriptSessionCache = update.order;
  const removed = new Set(update.evicted);
  for (const sessionId of state.timelines.keys()) {
    if (!valid.has(sessionId)) removed.add(sessionId);
  }
  evictTranscriptSessions([...removed]);
}

// --- Reconnect bookkeeping --------------------------------------------------
// The banner + version-probe + outage/reconnect-flash policy now live in `conn`
// (the shared app-shell connection store). The store just reports socket
// open/close into it and reads back the backoff delay. The only reconnect state
// kept here is the single pending-attempt timer.
let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
let outageStartedAt: number | undefined;
let reconnectAttempts = 0;
let nextConnectReason = "initial";

function clearReconnectTimer(): void {
  if (reconnectTimer !== undefined) {
    clearTimeout(reconnectTimer);
    reconnectTimer = undefined;
  }
}

function abandonProductSocket(): void {
  productSessionAbandoned = true;
  clearReconnectTimer();
  stopLiveness();
  const current = socket;
  socket = undefined;
  if (state.connected) setState({ ...state, connected: false });
  current?.close();
}

// --- Liveness watchdog (half-open detection) --------------------------------
// A WebSocket can go HALF-OPEN — TCP still "connected" but no data flows and
// `onclose` NEVER fires — which is common on mobile/5G (NAT drops, radio
// handoffs, app suspend). The socket then silently stops delivering updates and
// the UI freezes at the last-seen state (e.g. a status stuck mid-turn → a
// spinner that never resolves). The daemon sends an app-level heartbeat
// (Outbound::Ping) on a fixed interval, so on a HEALTHY socket some message
// always arrives within it; prolonged silence means the socket is dead. We
// track the last-message time and, if it goes stale, replace it immediately →
// fresh snapshot (self-healing the frozen state without waiting for a mobile
// browser's close-handshake timeout).
const STALE_MS = 60_000; // ~2.4 missed 25s heartbeats → dead (conservative)
const FOREGROUND_STALE_MS = 30_000; // on app-foreground, suspend likely killed it
const FOREGROUND_RECOVERY_COALESCE_MS = 1_000;
const LIVENESS_CHECK_MS = 15_000;
let lastMessageAt = 0;
let lastForegroundRecoveryAt = 0;
let livenessTimer: ReturnType<typeof setInterval> | undefined;

function markAlive(): void {
  lastMessageAt = Date.now();
}
function stopLiveness(): void {
  if (livenessTimer !== undefined) {
    clearInterval(livenessTimer);
    livenessTimer = undefined;
  }
}
function startLiveness(ws: WebSocket): void {
  stopLiveness();
  livenessTimer = setInterval(() => {
    if (
      socket === ws && ws.readyState === WebSocket.OPEN &&
      Date.now() - lastMessageAt > STALE_MS
    ) {
      reconnectNow("liveness_stale");
    }
  }, LIVENESS_CHECK_MS);
}

// A suspended mobile WebSocket can remain OPEN in JS while its underlying TCP
// connection is already gone. Calling close() and waiting for onclose makes
// foreground recovery depend on the browser's close-handshake timeout. Detach
// the zombie first so its eventual callbacks cannot affect the replacement,
// then open the replacement immediately. This path is user-driven (foreground
// or network return), so it intentionally bypasses outage backoff.
function reconnectNow(reason: string): void {
  if (productSessionAbandoned) return;
  const stale = socket;
  if (!shouldStartImmediateReconnect(stale?.readyState)) {
    reportClientLog("info", "websocket_reconnect_coalesced", "Cowboy WebSocket reconnect coalesced", {
      reason,
      ready_state: stale?.readyState ?? -1,
      visibility: document.visibilityState,
      network_online: navigator.onLine,
    });
    clearReconnectTimer();
    return;
  }
  const silenceMs = lastMessageAt > 0 ? Math.max(0, Date.now() - lastMessageAt) : 0;
  reportClientLog("warn", "websocket_reconnect_triggered", "Cowboy WebSocket reconnect triggered", {
    reason,
    ready_state: stale?.readyState ?? -1,
    silence_ms: silenceMs,
    visibility: document.visibilityState,
    network_online: navigator.onLine,
  });
  outageStartedAt ??= Date.now();
  socket = undefined;
  clearReconnectTimer();
  stopLiveness();
  if (state.connected) setState({ ...state, connected: false });
  stale?.close();
  nextConnectReason = reason;
  openSocket();
}

// Mobile suspends a backgrounded tab (freezing timers AND often killing the
// socket without an `onclose`); on return the socket may be a zombie. Timers
// were frozen, so the watchdog above hasn't run — repaint pending canonical
// state immediately, then probe the socket and reconnect if it has gone stale.
// Keep the liveness and reconnect policy unchanged while hidden so background
// turn notifications retain their existing delivery behaviour.
if (typeof document !== "undefined") {
  const appleTouchWebView = isAppleTouchWebView(
    globalThis.navigator?.userAgent ?? "",
    globalThis.navigator?.platform ?? "",
    globalThis.navigator?.maxTouchPoints ?? 0,
  );
  const recoverForeground = (): void => {
    if (document.visibilityState !== "visible") return;
    if (notifyScheduled) flushNotify();
    const now = Date.now();
    // iOS can dispatch both visibilitychange and pageshow for one foreground
    // transition. Coalesce them so the forced Apple reconnect never replaces
    // its own in-flight successor.
    if (now - lastForegroundRecoveryAt < FOREGROUND_RECOVERY_COALESCE_MS) return;
    if (
      shouldReconnectOnForeground(
        socket?.readyState,
        now - lastMessageAt,
        FOREGROUND_STALE_MS,
        appleTouchWebView,
      )
    ) {
      lastForegroundRecoveryAt = now;
      reconnectNow(appleTouchWebView ? "apple_foreground" : "foreground_stale");
    }
  };
  document.addEventListener("visibilitychange", recoverForeground);
  globalThis.addEventListener("pageshow", recoverForeground);
  globalThis.addEventListener("online", () => reconnectNow("network_online"));
  globalThis.addEventListener("cowboy:product-sign-out", abandonProductSocket);
}

function emit(): void {
  scheduleNotify();
}

// WebSocket chunks can arrive much faster than the display can paint. State is
// still reduced synchronously (no event is lost), but visible subscriber
// rendering keeps the established ~20fps cadence and ~10fps while scrolling.
// In a hidden tab, observers receive a coarse 1fps flush instead of repeatedly
// rendering an invisible tree; foregrounding flushes pending canonical state
// immediately above. Alert delivery remains synchronous in `handle`.
let notifyScheduled = false;
let lastNotifyAt = 0;
let notifyTimer: ReturnType<typeof setTimeout> | undefined;
let notifyFrame = 0;
let presentationHoldCount = 0;
function flushNotify(): void {
  if (notifyTimer !== undefined) {
    clearTimeout(notifyTimer);
    notifyTimer = undefined;
  }
  if (notifyFrame !== 0) {
    cancelAnimationFrame(notifyFrame);
    notifyFrame = 0;
  }
  if (!notifyScheduled) return;
  // A compositor-owned workspace gesture is presenting one stable raster.
  // Continue reducing canonical state, but do not let any React subscriber
  // mutate the visible tree until the gesture and its settle animation finish.
  if (presentationHoldCount > 0) return;
  notifyScheduled = false;
  presentedState = state;
  lastNotifyAt = performance.now();
  for (const listener of listeners) listener();
}

/** Freeze visible store subscribers while canonical websocket state continues
 * reducing. Reference-counted so a gesture that begins during another release
 * window cannot unfreeze the workspace early. Releasing coalesces the entire
 * interval into one latest-snapshot notification. */
export function holdStorePresentation(): () => void {
  presentationHoldCount += 1;
  let released = false;
  return (): void => {
    if (released) return;
    released = true;
    presentationHoldCount = Math.max(0, presentationHoldCount - 1);
    if (presentationHoldCount === 0 && notifyScheduled) {
      scheduleNotifyFrame();
    }
  };
}

/** Canonical timeline access for the transcript's post-freeze catch-up. Unlike
 * the React selector snapshot, this is current even while notifications are
 * intentionally held. */
export function canonicalTimeline(sessionId: string): Envelope[] | undefined {
  return state.timelines.get(sessionId);
}
function scheduleNotifyFrame(): void {
  if (notifyFrame !== 0) return;
  notifyFrame = requestAnimationFrame(() => {
    notifyFrame = 0;
    flushNotify();
  });
}
function scheduleNotify(): void {
  if (notifyScheduled) return;
  notifyScheduled = true;
  if (typeof document !== "undefined" && document.visibilityState === "visible") {
    const remaining = Math.max(
      0,
      transcriptPresentationIntervalMs() - (performance.now() - lastNotifyAt),
    );
    if (remaining === 0) {
      scheduleNotifyFrame();
    } else {
      notifyTimer = setTimeout(() => {
        notifyTimer = undefined;
        scheduleNotifyFrame();
      }, remaining);
    }
  } else {
    notifyTimer = setTimeout(() => {
      notifyTimer = undefined;
      flushNotify();
    }, 1000);
  }
}

function setState(next: State): void {
  state = next;
  emit();
}

const NETWORK_ACTION_TIMEOUT_MS = 10_000;

/** Resolve a UI mutation only after the authoritative store reflects it.
 * Buttons use this promise for truthful delayed progress: a fast round-trip
 * never shows a spinner, while a slow one stays disabled until the matching
 * broadcast/echo arrives. */
function waitForState(
  predicate: (snapshot: State) => boolean,
  label: string,
): Promise<void> {
  if (predicate(state)) return Promise.resolve();
  return new Promise((resolve, reject) => {
    let timeout = 0;
    const done = (): void => {
      listeners.delete(check);
      globalThis.clearTimeout(timeout);
      resolve();
    };
    const check = (): void => {
      if (predicate(state)) done();
    };
    listeners.add(check);
    timeout = globalThis.setTimeout(() => {
      listeners.delete(check);
      reject(new Error(`${label} was not acknowledged`));
    }, NETWORK_ACTION_TIMEOUT_MS);
    // Close the tiny send/register race against a synchronous test transport.
    check();
  });
}

function sendWithAck(
  command: Inbound,
  predicate: (snapshot: State) => boolean,
  label: string,
): Promise<void> {
  if (!send(command)) {
    return Promise.reject(new Error(`${label} is unavailable while reconnecting`));
  }
  return waitForState(predicate, label);
}

// --- Server-synced queue + drafts -------------------------------------------
//
// The daemon is authoritative: it sends the full queue + drafts for a session in
// a `queues` broadcast (on connect and after every change), and every mutation
// is a command the store sends back. There is NO local persistence, drain, or
// serialization here anymore — those moved server-side so all terminals stay in
// sync (see src/core.rs). Build the ACP content blocks for a staged message
// exactly as a prompt would carry them (empty ⇒ plain text).
function contentOf(text: string, attachments: readonly Attachment[]): ContentBlock[] {
  return buildContentBlocks(text, attachments) ?? [];
}

// Convert a server `WireQueued` (raw ACP content blocks) into the UI shape, with
// attachments reconstructed for display / re-edit.
function fromWire(list: WireQueued[]): QueuedMessage[] {
  return list.map((m) => ({
    id: m.id,
    text: m.text,
    attachments: blocksToAttachments(m.content, m.text),
    // Carried so an optimistic row can reconcile against its confirmed twin by
    // id. `?? undefined` keeps `exactOptionalPropertyTypes` happy.
    ...(m.cmid !== undefined && { cmid: m.cmid }),
    ...(m.schedule !== undefined && { schedule: m.schedule }),
  }));
}

function sameQueuedContent(
  left: QueuedMessage,
  right: QueuedMessage,
): boolean {
  return left.text === right.text &&
    JSON.stringify(contentOf(left.text, left.attachments)) ===
      JSON.stringify(contentOf(right.text, right.attachments));
}

// A canonical live envelope may absorb many raw sequence numbers. Keep those
// numbers out-of-band so reconnect snapshots can still deduplicate their raw
// overlap without retaining every large payload. Weak keys disappear with the
// timeline rows they describe.
const absorbedSeqs = new WeakMap<Envelope, Set<number>>();

function absorbed(previous: Envelope, replacement: Envelope, seq: number): Envelope {
  const seen = absorbedSeqs.get(previous) ?? new Set([previous.seq]);
  seen.add(seq);
  absorbedSeqs.set(replacement, seen);
  return replacement;
}

function containsSeq(events: Envelope[], seq: number): boolean {
  const last = events[events.length - 1];
  if (last && absorbedSeqs.get(last)?.has(seq) === true) return true;
  // Live traffic is monotonic. Avoid scanning the whole visible history for
  // the overwhelmingly common append case; reconnect/history overlap falls
  // through to the full check below.
  if (last && seq > last.seq) return false;
  return events.some((event) => event.seq === seq || absorbedSeqs.get(event)?.has(seq) === true);
}

function findLatestTool(events: Envelope[], toolId: string): number {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const candidate = events[index];
    if (
      candidate?.kind === "update" &&
      candidate.update.sessionUpdate === "tool_call" &&
      candidate.update.toolCallId === toolId
    ) return index;
  }
  return -1;
}

function applyEnvelope(timelines: Map<string, Envelope[]>, env: Envelope): Map<string, Envelope[]> {
  const existing = timelines.get(env.session_id) ?? [];
  if (containsSeq(existing, env.seq)) return timelines; // dedup

  // Clear is a destructive transcript boundary, not an ordinary divider.
  // Replace the local history immediately; the daemon deletes the same rows
  // durably so a reload cannot restore them.
  if (
    env.kind === "update" &&
    env.update.sessionUpdate === "context_cleared"
  ) {
    const next = new Map(timelines);
    next.set(env.session_id, [env]);
    return next;
  }

  // These high-frequency frames are runtime telemetry, not transcript history.
  // The daemon persists their sequence watermark without storing a row; mirror
  // that canonical representation in the live client so long turns don't grow
  // one inert object per token update.
  if (
    env.kind === "update" &&
    (env.update.sessionUpdate === "usage_update" ||
      env.update.sessionUpdate === "session_info_update" ||
      isPureTerminalOutputDelta(env.update))
  ) {
    return timelines;
  }

  let merged: Envelope[] | undefined;
  if (env.kind === "update") {
    const updateKind = env.update.sessionUpdate;
    if (
      updateKind === "agent_message_chunk" ||
      updateKind === "agent_thought_chunk" ||
      updateKind === "user_message_chunk"
    ) {
      const last = existing[existing.length - 1];
      if (
        last?.kind === "update" &&
        last.update.sessionUpdate === updateKind &&
        last.update.messageId === env.update.messageId &&
        last.update.content?.type === "text" &&
        env.update.content?.type === "text"
      ) {
        const replacement: Envelope = {
          ...last,
          update: {
            ...last.update,
            content: {
              ...last.update.content,
              text: `${last.update.content.text ?? ""}${env.update.content.text ?? ""}`,
            },
          },
        };
        merged = linkTimeline(
          [...existing.slice(0, -1), absorbed(last, replacement, env.seq)],
          existing,
        );
      }
    } else if (updateKind === "tool_call_update") {
      const toolId = env.update.toolCallId;
      const index = toolId ? findLatestTool(existing, toolId) : -1;
      const initial = index >= 0 ? existing[index] : undefined;
      if (initial?.kind === "update") {
        const replacement: Envelope = {
          ...initial,
          update: {
            ...initial.update,
            ...env.update,
            sessionUpdate: "tool_call",
          },
        };
        merged = linkTimeline([...existing], existing);
        merged[index] = absorbed(initial, replacement, env.seq);
      }
    }
  }

  merged ??= linkTimeline(
    existing.length === 0 || env.seq > (existing[existing.length - 1]?.seq ?? 0)
      ? [...existing, env]
      : [...existing, env].sort((a, b) => a.seq - b.seq),
    existing,
  );
  const next = new Map(timelines);
  next.set(env.session_id, merged);
  return next;
}

// Merge two seq-ordered runs in linear time while preserving object identity
// for existing events. Scrollback used to scan the whole window for every
// incoming event and then sort the combined window on every page, making
// progressively older history increasingly expensive.
function mergeEvents(
  timelines: Map<string, Envelope[]>,
  sessionId: string,
  events: Envelope[],
): Map<string, Envelope[]> {
  const existing = timelines.get(sessionId) ?? [];
  const merged = mergeCanonicalTimeline(existing, events);
  if (
    merged.length === existing.length &&
    merged.every((event, index) => event === existing[index])
  ) {
    return timelines;
  }
  const next = new Map(timelines);
  next.set(sessionId, linkTimeline(merged, existing));
  return next;
}

// Fetch the next OLDER page of a session's history and prepend it. Pages come
// from the immutable HTTP route (GET /api/history/:id) so a re-fetch
// (scroll back, reload, post-recycle) is a cache hit — zero network. One fetch
// at a time per session; no-op once the window already reaches the first event.
//
// VERSION-SCOPED url (`?v=<build>`): the cache is `immutable`, so without this a
// redeploy could keep serving pages cached under the OLD build (if a future
// version ever changes the event shape, that's stale/incompatible). Keying the
// url on the build id means a new version's pages are fresh fetches, while
// reloads of the SAME build stay cache hits. (`conn.version()` is the id the tab
// loaded against; until the first /version probe lands it's a harmless "0".)
const HISTORY_FETCH_TIMEOUT_MS = 6_000;

export async function loadOlder(sessionId: string): Promise<boolean> {
  const pg = state.pagination.get(sessionId);
  if (!pg || pg.reachedStart || pg.loadingOlder || pg.beforeSeq === null) return false;
  const epoch = transcriptEpoch.get(sessionId) ?? 0;
  const beforeSeq = pg.beforeSeq;
  const controller = new AbortController();
  const timeout = globalThis.setTimeout(
    () => controller.abort("history request timed out"),
    HISTORY_FETCH_TIMEOUT_MS,
  );
  setPagination(sessionId, { ...pg, loadingOlder: true });
  try {
    const res = await fetch(
      `/api/history/${encodeURIComponent(sessionId)}?before_seq=${String(beforeSeq)}&v=${encodeURIComponent(conn.version() ?? "0")}`,
      { signal: controller.signal },
    );
    if (!res.ok) {
      setPagination(sessionId, { ...pg, loadingOlder: false });
      return false;
    }
    const data = (await res.json()) as {
      events: Envelope[];
      next_before_seq: number | null;
      reached_start: boolean;
    };
    if ((transcriptEpoch.get(sessionId) ?? 0) !== epoch) return false;
    setState({ ...state, timelines: mergeEvents(state.timelines, sessionId, data.events) });
    // Always step to the next OLDER page (don't recompute from the oldest seq —
    // a gap at a boundary would re-request the same page forever). Page 0 was
    // the last → reached start.
    setPagination(sessionId, {
      reachedStart: data.reached_start,
      loadingOlder: false,
      beforeSeq: data.next_before_seq,
    });
    return data.reached_start || data.next_before_seq !== beforeSeq ||
      data.events.length > 0;
  } catch {
    setPagination(sessionId, { ...pg, loadingOlder: false });
    return false;
  } finally {
    globalThis.clearTimeout(timeout);
  }
}

export async function loadPreviousQuestionPage(sessionId: string): Promise<boolean> {
  const pg = state.pagination.get(sessionId);
  if (!pg || pg.reachedStart || pg.loadingOlder || pg.beforeSeq === null) return false;
  const epoch = transcriptEpoch.get(sessionId) ?? 0;
  const beforeSeq = pg.beforeSeq;
  setPagination(sessionId, { ...pg, loadingOlder: true });
  try {
    const res = await fetch(
      `/api/history/${encodeURIComponent(sessionId)}?before_seq=${String(beforeSeq)}&question_page=true&v=${encodeURIComponent(conn.version() ?? "0")}`,
    );
    if (!res.ok) {
      setPagination(sessionId, { ...pg, loadingOlder: false });
      return false;
    }
    const data = (await res.json()) as {
      events: Envelope[];
      next_before_seq: number | null;
      reached_start: boolean;
    };
    if ((transcriptEpoch.get(sessionId) ?? 0) !== epoch) return false;
    setState({
      ...state,
      timelines: mergeEvents(state.timelines, sessionId, data.events),
    });
    setPagination(sessionId, {
      reachedStart: data.reached_start,
      loadingOlder: false,
      beforeSeq: data.next_before_seq,
    });
    return data.events.length > 0;
  } catch {
    setPagination(sessionId, { ...pg, loadingOlder: false });
    return false;
  }
}

export async function loadQuestionPage(
  sessionId: string,
  pageId: string,
): Promise<boolean> {
  const epoch = transcriptEpoch.get(sessionId) ?? 0;
  const rootSeq = Number(pageId);
  if (!Number.isSafeInteger(rootSeq) || rootSeq < 0) return false;
  if (completeQuestionPages.get(sessionId)?.has(pageId)) return true;
  try {
    const response = await fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/question-pages/${encodeURIComponent(pageId)}?v=${
        encodeURIComponent(conn.version() ?? "0")
      }`,
      // The newest page may still be growing. Never let WebKit reuse a partial
      // response captured between the user prompt and the agent answer.
      { cache: "no-store" },
    );
    if (!response.ok) return false;
    const data = (await response.json()) as { events: Envelope[] };
    if ((transcriptEpoch.get(sessionId) ?? 0) !== epoch) return false;
    const complete = data.events.some((event) => event.seq === rootSeq);
    if (complete) {
      const pages = completeQuestionPages.get(sessionId) ?? new Set<string>();
      pages.add(pageId);
      completeQuestionPages.set(sessionId, pages);
    }
    // Publish completeness before notifying store subscribers. Page View reads
    // this side cache during the render caused by setState; doing it afterwards
    // left the completed response behind an eternal skeleton until some
    // unrelated state change (most visibly switching sessions) rendered again.
    setState({
      ...state,
      timelines: mergeEvents(state.timelines, sessionId, data.events),
    });
    return complete;
  } catch {
    return false;
  }
}

const completeQuestionPages = new Map<string, Set<string>>();
const transcriptEpoch = new Map<string, number>();

export function isQuestionPageLoaded(sessionId: string, pageId: string): boolean {
  return completeQuestionPages.get(sessionId)?.has(pageId) === true;
}

const INACTIVE_HISTORY_TAIL = 800;
const INACTIVE_HISTORY_HIGH_WATER = 1_000;
// The open transcript used to grow forever between page reloads. Keep a smaller
// recent window once a following reader crosses this batched high-water mark.
// The complete log remains in Postgres and `loadOlder` pages it back on demand.
const ACTIVE_HISTORY_TAIL = 120;
const ACTIVE_HISTORY_HIGH_WATER = 200;
const ACTIVE_HISTORY_RENDER_ROWS = 48;

function releaseHistoryTail(sessionId: string, retain: number): boolean {
  const timeline = state.timelines.get(sessionId);
  if (!timeline || timeline.length <= retain) return false;
  const retained = retainTimelineState(timeline, retain);
  // Preserve render-item identity across the trim for exactly one derivation.
  // Without this link every surviving row receives a fresh RenderItem object,
  // so React re-runs every visible Markdown tree at once — perceived as a full
  // Conversation flash during a streaming turn. `derive()` consumes and deletes
  // this parent link immediately; the discarded deep history is still
  // collectible after that render (there is no ancestry chain).
  const retainedTimeline = linkTimeline(retained.events, timeline);
  // A retained root event does not prove that the rest of its immutable page
  // survived this timeline trim. Force the next Page View visit through the
  // page endpoint instead of presenting a root-only, falsely non-scrollable
  // answer.
  completeQuestionPages.delete(sessionId);
  const timelines = new Map(state.timelines);
  timelines.set(sessionId, retainedTimeline);
  const pagination = new Map(state.pagination);
  pagination.set(sessionId, {
    reachedStart: false,
    loadingOlder: false,
    beforeSeq: retained.recentStartSeq,
  });
  setState({ ...state, timelines, pagination });
  return true;
}

/** Release deep scrollback only while the reader follows the live edge.
 * Detached readers keep their complete visible window until they return. */
export function releaseFollowedHistory(sessionId: string): boolean {
  const timeline = state.timelines.get(sessionId);
  if (!timeline || timeline.length <= ACTIVE_HISTORY_HIGH_WATER) return false;
  // A raw ACP envelope count is not a visual-row count. Preserve enough event
  // history to keep the latest real rows mounted; otherwise a tool-heavy turn
  // can shrink below one viewport and the auto-fill loader immediately restores
  // the just-trimmed page, creating a permanent trim/load flash loop.
  const retain = retainedEventCountForRows(
    timeline,
    ACTIVE_HISTORY_TAIL,
    ACTIVE_HISTORY_RENDER_ROWS,
  );
  return releaseHistoryTail(sessionId, retain);
}

/** Release deep scrollback after leaving a session. Recent context remains
 * instant; immutable cursor pages can be fetched again if the user scrolls. */
export function releaseInactiveHistory(sessionId: string): void {
  const timeline = state.timelines.get(sessionId);
  if (!timeline || timeline.length <= INACTIVE_HISTORY_TAIL) return;
  releaseHistoryTail(sessionId, INACTIVE_HISTORY_TAIL);
}

function setPagination(
  sessionId: string,
  value: { reachedStart: boolean; loadingOlder: boolean; beforeSeq: number | null },
): void {
  const pagination = new Map(state.pagination);
  pagination.set(sessionId, value);
  setState({ ...state, pagination });
}

function handle(msg: Outbound): void {
  switch (msg.type) {
    case "ping":
      // Heartbeat: its ARRIVAL is the signal (onmessage stamps lastMessageAt for
      // the liveness watchdog). Nothing to render.
      break;
    case "sessions": {
      // No alert here: a status flip (busy → running) fired on mid-turn churn and
      // missed permission pauses. The chime/vibration is now driven precisely by
      // the `turn_end` + `permission_request` events in `case "event"` below.
      // Commit the list — the queue drain is now server-side, so there's no
      // client-side in-flight reconciliation to do. `sessionsLoaded` latches true
      // on the first list so the UI can detect a now-gone persisted focus.
      // Keep the raw list; the rendered `sessions` is re-derived with the title +
      // order overlays so a synced rename/reorder isn't clobbered by an unrelated
      // `sessions` broadcast (status flip, new/deleted session). The overlays are
      // the client's source of truth for title + order.
      rawSessions = msg.sessions;
      if (!state.sessionsLoaded) setState({ ...state, sessionsLoaded: true });
      commitSessions();
      const machines = projectMachineOccupancy(state.machines, msg.sessions);
      if (machines !== state.machines) setState({ ...state, machines });
      // The list is authoritative: drop composer drafts for sessions that no
      // longer exist (deleted here or on another terminal). Tolerant + off the
      // input path.
      const validSessions = new Set(msg.sessions.map((s) => s.id));
      pruneDrafts(validSessions);
      prunePendingEdits(validSessions);
      retainTranscriptSessions(validSessions);
      break;
    }
    case "machines": {
      if (!acceptsMachineSnapshot(
        state.machinesRevision,
        state.machinesLoaded,
        msg.revision,
        msg.resync === true,
      )) break;
      setState({
        ...state,
        machines: projectMachineOccupancy(msg.machines, state.sessions),
        machinesLoaded: true,
        machinesRevision: msg.revision,
      });
      break;
    }
    case "snapshot": {
      // A hydration response can race an LRU eviction. Reopening the session
      // starts a fresh bootstrap; retaining this stale response would defeat the
      // cache bound and can overwrite a newer transcript epoch.
      if (!transcriptIsCached(msg.session_id)) break;
      const timelines = mergeEvents(state.timelines, msg.session_id, msg.events);
      // Mark hydrated even when `events` is empty: the snapshot's arrival IS the
      // "history loaded" signal. Reconnects re-send snapshots but the flag stays
      // set (cached history shown, no skeleton flash). Reuse the same Set when
      // already present so the snapshot reference stays stable.
      const hydrated = state.hydrated.has(msg.session_id)
        ? state.hydrated
        : new Set(state.hydrated).add(msg.session_id);
      // Seed pagination on the FIRST snapshot only — a reconnect re-sends the
      // tail, but if we'd already paged older history in we must keep that
      // window's `reachedStart`, not reset it to the tail's.
      const pagination = state.pagination.has(msg.session_id)
        ? state.pagination
        : new Map(state.pagination).set(msg.session_id, {
            reachedStart: msg.reached_start,
            loadingOlder: false,
            // First older page = the one containing (oldestSeq − 1), so its lower
            // part (if any) is filled before going further back.
            beforeSeq: msg.events[0]?.seq ?? null,
          });
      const confirmedCmids = new Set(
        msg.events.flatMap((event) => event.cmid === undefined ? [] : [event.cmid]),
      );
      const optimisticMessages = confirmedCmids.size === 0
        ? state.optimisticMessages
        : reconcileOptimistic(
          state.optimisticMessages,
          msg.session_id,
          confirmedCmids,
        );
      setState({
        ...state,
        timelines,
        hydrated,
        pagination,
        optimisticMessages,
      });
      // A reload may hydrate the durable outbox before this bootstrap arrives.
      // If the daemon had already accepted the prompt, the snapshot's user echo
      // is the acknowledgement: retire the persisted mutation so reconnect
      // cannot resend it or paint a duplicate optimistic bubble.
      if (confirmedCmids.size > 0) {
        const client = qClients.get(msg.session_id);
        const confirmedMutations = (client?.pending() ?? []).filter((mutation) => {
          const rowCmid = (mutation.args as { row?: QueuedMessage }).row?.cmid;
          return confirmedCmids.has(mutation.id) ||
            (rowCmid !== undefined && confirmedCmids.has(rowCmid));
        });
        if (client !== undefined && confirmedMutations.length > 0) {
          for (const mutation of confirmedMutations) {
            const rowCmid = (mutation.args as { row?: QueuedMessage }).row?.cmid ??
              mutation.id;
            suppressedInFlight.add(rowCmid);
            clearOptTimers(mutation.id);
            qStatus.delete(mutation.id);
            qStatus.delete(rowCmid);
          }
          client.confirm(confirmedMutations.map((mutation) => mutation.id));
          commitQueue(msg.session_id);
          for (const mutation of confirmedMutations) {
            const rowCmid = (mutation.args as { row?: QueuedMessage }).row?.cmid ??
              mutation.id;
            suppressedInFlight.delete(rowCmid);
          }
        }
      }
      const retry = sessionHydrationRetryTimers.get(msg.session_id);
      if (retry !== undefined) clearTimeout(retry);
      sessionHydrationRetryTimers.delete(msg.session_id);
      break;
    }
    case "event": {
      const env = msg.envelope;
      const clearsContext = env.kind === "update" &&
        env.update.sessionUpdate === "context_cleared";
      // Attention alert — a permission request needs a decision. Snapshot/history
      // replays take another reducer path, so past requests never re-alert.
      if (env.kind === "permission_request") fireAlert("decision");
      // The dispatched prompt's user-echo carries the originating client's cmid
      // → CONFIRMS the optimistic chat bubble. Cross-signal: a submit GUESSED as
      // a chat send but actually QUEUED is confirmed here too — so also drop that
      // cmid from the queue sync client's pending (the wrong-guess self-heal).
      const cmid = env.cmid;
      // Only touch the queue when this cmid is ACTUALLY a queue pending (a
      // wrong-guessed submit that dispatched). Guarding avoids a spurious
      // queues/drafts reference churn on every chat echo — keeps the immutable
      // transcript optimization's referential stability intact.
      const qc = cmid === undefined ? undefined : qClients.get(env.session_id);
      if (cmid !== undefined && qc !== undefined && qc.pending().some((m) => m.id === cmid)) {
        qc.confirm([cmid]);
        if (qStatus.has(cmid)) {
          clearOptTimers(cmid);
          qStatus.delete(cmid);
        }
        commitQueue(env.session_id);
      }
      const pagination = clearsContext
        ? new Map(state.pagination).set(env.session_id, {
            reachedStart: true,
            loadingOlder: false,
            beforeSeq: null,
          })
        : state.pagination;
      let optimisticMessages = state.optimisticMessages;
      if (clearsContext) {
        resetExploreAfterContextClear(env.session_id);
        completeQuestionPages.delete(env.session_id);
        transcriptEpoch.set(
          env.session_id,
          (transcriptEpoch.get(env.session_id) ?? 0) + 1,
        );
        if (optimisticMessages.has(env.session_id)) {
          optimisticMessages = new Map(optimisticMessages);
          optimisticMessages.delete(env.session_id);
        }
      }
      setState({
        ...state,
        // Live fan-out covers every running session. Only the MRU working set
        // owns transcript payloads; inactive evictions rehydrate on demand.
        timelines: transcriptIsCached(env.session_id)
          ? applyEnvelope(state.timelines, env)
          : state.timelines,
        pagination,
        optimisticMessages,
        ...(cmid !== undefined && {
          optimisticMessages: reconcileOptimistic(state.optimisticMessages, env.session_id, new Set([cmid])),
        }),
      });
      if (
        env.session_id !== openedSessionId &&
        (state.timelines.get(env.session_id)?.length ?? 0) > INACTIVE_HISTORY_HIGH_WATER
      ) {
        releaseHistoryTail(env.session_id, INACTIVE_HISTORY_TAIL);
      }
      break;
    }
    case "config_options": {
      configOptionsRevisions.set(
        msg.session_id,
        (configOptionsRevisions.get(msg.session_id) ?? 0) + 1,
      );
      const next = new Map(state.configOptions);
      next.set(msg.session_id, msg.options);
      setState({ ...state, configOptions: next });
      break;
    }
    case "sync_patch": {
      // Arbiter snapshot for one state: fold it into that state's sync client
      // (keeps the highest version unless `resync` forces it, drops confirmed
      // pending, rebases the rest). Per-session queue states route to the queue
      // path; title/order to their registered clients. Unknown → ignored.
      const resync = msg.resync === true;
      if (msg.state.startsWith("queue:")) {
        applyQueuePatch(msg.state.slice("queue:".length), msg.version, msg.value, msg.confirmed, resync);
      } else {
        if (msg.state.startsWith("mobile-review:")) {
          mobileReviewClient(msg.state.slice("mobile-review:".length));
        }
        syncClients.get(msg.state)?.applyPatch(msg.version, msg.value, msg.confirmed, resync);
      }
      break;
    }
    case "settings": {
      // Compatibility tombstone from controllers spanning the rollout.
      break;
    }
    case "error": {
      errorSeq += 1;
      // exactOptionalPropertyTypes: only include sessionId if non-undefined.
      const notice: ErrorNotice =
        msg.session_id !== undefined
          ? { seq: errorSeq, sessionId: msg.session_id, message: msg.message }
          : { seq: errorSeq, message: msg.message };
      console.warn("cowboy error:", msg.message);
      setState({ ...state, lastError: notice });
      // Something went wrong with an agent turn → the error chime (only for
      // session-scoped errors; a bare command rejection isn't a "problem" worth a
      // sound). `fireAlert` still self-gates on the setting + tab visibility.
      if (msg.session_id !== undefined) {
        fireAlert("error");
        if (vibrateAlertOn()) notifyHaptic("error");
      }
      break;
    }
  }
}

async function hydrateSession(
  sessionId: string,
  replace = false,
  retryAttempt = 0,
): Promise<void> {
  const existing = sessionHydrations.get(sessionId);
  if (existing && !replace) return existing.promise;
  existing?.controller.abort();
  const scheduledRetry = sessionHydrationRetryTimers.get(sessionId);
  if (scheduledRetry !== undefined) clearTimeout(scheduledRetry);
  sessionHydrationRetryTimers.delete(sessionId);
  const controller = new AbortController();
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, 8000);
  const configOptionsRevisionAtRequestStart =
    configOptionsRevisions.get(sessionId) ?? 0;
  const promise = (async (): Promise<void> => {
    let retryableFailure = false;
    try {
      const response = await fetch(
        `/api/sessions/${encodeURIComponent(sessionId)}/bootstrap`,
        { cache: "no-store", signal: controller.signal },
      );
      if (!response.ok) {
        throw new Error(`session bootstrap failed: ${String(response.status)}`);
      }
      const bootstrap = (await response.json()) as SessionBootstrapResponse;
      const applyHydratedConfigOptions = shouldApplyHydratedConfigOptions(
        configOptionsRevisionAtRequestStart,
        configOptionsRevisions.get(sessionId) ?? 0,
      );
      for (const message of bootstrap.messages) {
        if (message.type === "config_options" && !applyHydratedConfigOptions) {
          continue;
        }
        handle(message);
      }
      if (!state.hydrated.has(sessionId)) {
        throw new Error("session bootstrap contained no transcript snapshot");
      }
    } catch (error) {
      retryableFailure = timedOut || !controller.signal.aborted;
      if (retryableFailure) console.warn("session bootstrap failed", error);
    } finally {
      clearTimeout(timeout);
      if (sessionHydrations.get(sessionId)?.controller === controller) {
        sessionHydrations.delete(sessionId);
      }
      if (
        retryableFailure &&
        !state.hydrated.has(sessionId) &&
        transcriptIsCached(sessionId) &&
        openedSessionId === sessionId &&
        retryAttempt < SESSION_HYDRATION_RETRY_DELAYS_MS.length
      ) {
        const delay = SESSION_HYDRATION_RETRY_DELAYS_MS[retryAttempt]!;
        const retry = setTimeout(() => {
          if (sessionHydrationRetryTimers.get(sessionId) !== retry) return;
          sessionHydrationRetryTimers.delete(sessionId);
          if (
            !state.hydrated.has(sessionId) &&
            transcriptIsCached(sessionId) &&
            openedSessionId === sessionId
          ) {
            void hydrateSession(sessionId, true, retryAttempt + 1);
          }
        }, delay);
        sessionHydrationRetryTimers.set(sessionId, retry);
      }
    }
  })();
  sessionHydrations.set(sessionId, { promise, controller });
  return promise;
}

/** Retry only this session's bootstrap; never reload the whole application or
 * discard a local composer/delivery state merely because history was slow. */
export function retrySessionHydration(sessionId: string): Promise<void> {
  return hydrateSession(sessionId, true, 0);
}

/**
 * Surface a client-side notice through the same snackbar the daemon's "error"
 * messages use. `severity` defaults to "error"; pass "warning" for recoverable
 * conditions (e.g. a persisted focus session that no longer exists on reload).
 */
export function notify(message: string, severity: "error" | "warning" = "error"): void {
  errorSeq += 1;
  console.warn("cowboy notice:", message);
  setState({ ...state, lastError: { seq: errorSeq, message, severity } });
}

// Schedule the next reconnect after `delay` ms (the backoff `conn.connectionLost`
// computed off the consecutive-failure count). One pending attempt at a time.
const PRODUCT_AUTH_LOST_EVENT = "cowboy:product-auth-lost";
const WS_AUTH_REQUIRED_CLOSE_CODE = 4001;

type MeHandshake = "reconnect" | "logout" | "keep";

function classifyMeHandshake(status: number | "network"): MeHandshake {
  if (status === 200) return "reconnect";
  if (status === 401 || status === 403) return "logout";
  return "keep";
}

async function probeProductAuth(): Promise<MeHandshake> {
  try {
    const response = await fetch("/api/auth/me", {
      cache: "no-store",
      credentials: "same-origin",
      headers: { accept: "application/json" },
    });
    return classifyMeHandshake(response.status);
  } catch {
    return classifyMeHandshake("network");
  }
}

function logoutProductSession(): void {
  abandonProductSocket();
  globalThis.dispatchEvent(new Event(PRODUCT_AUTH_LOST_EVENT));
}

function scheduleReconnect(delay: number): void {
  if (productSessionAbandoned) return;
  if (reconnectTimer !== undefined) return;
  reportClientLog("info", "websocket_reconnect_scheduled", "Cowboy WebSocket reconnect scheduled", {
    delay_ms: delay,
    attempt: reconnectAttempts + 1,
  });
  reconnectTimer = setTimeout(() => {
    reconnectTimer = undefined;
    nextConnectReason = "backoff";
    connect();
  }, delay);
}

// Hydrate the locally-cached sync states (title/order) from IndexedDB ONCE, then
// open the socket. Instant-load: the last-known titles/order paint before the
// first server byte. Reconnects skip this — the in-memory state is already fresh
// and newer than the debounce-saved cache, so re-hydrating would stomp it.
let didHydrate = false;
function connect(): void {
  if (productSessionAbandoned) return;
  if (didHydrate) {
    openSocket();
    return;
  }
  didHydrate = true;
  // The hydrate is ONLY a cache-paint optimisation (last-known titles/order before
  // the first server byte); the socket must NEVER wait on it. A blocked IndexedDB
  // — e.g. another tab still holding an older DB version right after a deploy
  // reload — makes the open request HANG rather than reject, so a bare `await`
  // here would strand the app on "Connecting…" forever with no socket EVER
  // attempted (the symptom: app shell renders, WS never opens, no reconnect since
  // reconnect is driven by socket.onclose and there's no socket). So race the
  // hydrate against a short grace and open the socket on whichever finishes first.
  let opened = false;
  const openOnce = (): void => {
    if (opened) return;
    opened = true;
    openSocket();
  };
  const grace = setTimeout(openOnce, 1500);
  void (async (): Promise<void> => {
    try {
      // Queue outboxes carry authored prompts and must not wait behind an
      // unrelated title/order cache whose IndexedDB read is blocked. Hydrate
      // both classes concurrently; each client now merges safely if the socket
      // wins the grace race.
      await Promise.all([
        Promise.allSettled([...syncClients.values()].map((e) => e.hydrate())),
        hydrateCachedQueues(),
      ]);
      // If the 1.5s grace opened the socket before a slow IndexedDB read
      // completed, onopen could not see the restored outbox. Re-send now as
      // well. Mutation ids make the overlap with a simultaneous onopen safe and
      // idempotent; this closes the visible-pending-but-never-runs window.
      if (socket?.readyState === WebSocket.OPEN) {
        for (const entry of syncClients.values()) entry.resend();
        for (const store of qClients.values()) store.resend();
      }
    } catch (err) {
      console.warn("sync hydrate failed", err);
    } finally {
      clearTimeout(grace);
      openOnce();
    }
  })();
}

function openSocket(): void {
  if (productSessionAbandoned) return;
  // A reconnect can be triggered by several independent recovery paths
  // (backoff timer, foreground watchdog, connect guard). Never let them create
  // parallel sockets: a late close from an older socket would otherwise mark a
  // newer healthy connection as down and start an endless reconnect fan-out.
  if (
    socket?.readyState === WebSocket.OPEN ||
    socket?.readyState === WebSocket.CONNECTING
  ) {
    return;
  }
  const proto = globalThis.location.protocol === "https:" ? "wss:" : "ws:";
  const connectStartedAt = performance.now();
  const connectReason = nextConnectReason;
  const reconnecting = outageStartedAt !== undefined;
  if (reconnecting) reconnectAttempts += 1;
  reportClientLog("info", "websocket_connect_attempt", "Cowboy WebSocket connection attempt", {
    reason: connectReason,
    attempt: reconnecting ? reconnectAttempts : 0,
    reconnecting,
    network_online: navigator.onLine,
    visibility: document.visibilityState,
  });
  nextConnectReason = "unspecified";
  const ws = new WebSocket(`${proto}//${globalThis.location.host}/ws?bootstrap=lazy`);
  socket = ws;
  let openedAt: number | undefined;
  // A socket wedged in CONNECTING (a half-open proxy / network that completes the
  // TCP handshake but never the WS upgrade) fires NEITHER onopen NOR onclose, so
  // without this it strands the UI on "Connecting…" forever with no reconnect.
  // Force it closed after a grace → onclose → scheduleReconnect. Cleared the moment
  // it opens or closes on its own.
  const connectGuard = setTimeout(() => {
    if (socket === ws && ws.readyState === WebSocket.CONNECTING) {
      reportClientLog("warn", "websocket_connect_timeout", "Cowboy WebSocket upgrade timed out", {
        reason: connectReason,
        attempt: reconnecting ? reconnectAttempts : 0,
        timeout_ms: 8000,
      });
      ws.close();
    }
  }, 8000);
  ws.onopen = (): void => {
    clearTimeout(connectGuard);
    if (socket !== ws) {
      ws.close();
      return;
    }
    clearReconnectTimer();
    openedAt = performance.now();
    const connectDurationMs = openedAt - connectStartedAt;
    const outageDurationMs = outageStartedAt === undefined ? 0 : Math.max(0, Date.now() - outageStartedAt);
    reportClientMetric("websocket_connect_duration_ms", connectDurationMs, {
      connection: reconnecting ? "reconnect" : "initial",
    });
    if (reconnecting) {
      reportClientMetric("websocket_reconnect_duration_ms", outageDurationMs);
      reportClientMetric("websocket_reconnect_success", 1, { reason: connectReason });
    }
    reportClientLog("info", "websocket_open", "Cowboy WebSocket connected", {
      reason: connectReason,
      attempt: reconnecting ? reconnectAttempts : 0,
      reconnecting,
      connect_duration_ms: connectDurationMs,
      outage_duration_ms: outageDurationMs,
    });
    outageStartedAt = undefined;
    reconnectAttempts = 0;
    markAlive(); // seed liveness so the watchdog doesn't fire before the snapshot
    startLiveness(ws);
    setState({ ...state, connected: true });
    // Clears the failure count, flashes green if an outage was surfaced, and
    // probes /version for a redeploy (banner state lives in `conn`).
    conn.connectionReady();
    // Re-assert the open session so the daemon revives its agent if it died
    // with a restart we just reconnected across (revive-on-open, design §7).
    // Idempotent server-side when the agent is still alive.
    if (openedSessionId) {
      send({ type: "open_session", session_id: openedSessionId });
      void hydrateSession(openedSessionId, true);
    }
    // Re-send sync mutations the arbiter never confirmed (sent while the socket
    // was down). Each store re-sends its pending via its own `send` callback (a
    // generic `sync` frame for title/order, the add_draft/submit command for a
    // queue). The mutation id makes the daemon idempotent; the resync
    // `sync_patch` that follows drops them from pending once confirmed.
    for (const entry of syncClients.values()) entry.resend();
    for (const store of qClients.values()) store.resend();
    // The daemon pushes the session snapshot exactly once, right after connect.
    // If that first frame is somehow lost (a WKWebView reload race), the UI would
    // sit at "Loading…" forever — the socket is open, so no reconnect fires. Net:
    // if the snapshot hasn't landed a few seconds after open, close to force a
    // fresh reconnect (→ a fresh snapshot). No-op once any list has arrived.
    if (!state.sessionsLoaded) {
      setTimeout(() => {
        if (socket === ws && ws.readyState === WebSocket.OPEN && !state.sessionsLoaded) {
          ws.close();
        }
      }, 6000);
    }
  };
  ws.onmessage = (e: MessageEvent<string>): void => {
    if (socket !== ws) return;
    markAlive(); // any frame (incl. the heartbeat) proves the socket is alive
    try {
      handle(JSON.parse(e.data) as Outbound);
    } catch (err) {
      console.warn("bad message", err);
    }
  };
  ws.onclose = (event): void => {
    clearTimeout(connectGuard);
    // A superseded socket may close after its replacement has opened. It no
    // longer owns global connection state and must not raise the red banner or
    // schedule another reconnect.
    if (socket !== ws) return;
    socket = undefined;
    stopLiveness();
    setState({ ...state, connected: false });
    outageStartedAt ??= Date.now();
    reportClientLog("warn", "websocket_close", "Cowboy WebSocket disconnected", {
      code: event.code,
      clean: event.wasClean,
      socket_lifetime_ms: openedAt === undefined ? 0 : Math.max(0, performance.now() - openedAt),
      visibility: document.visibilityState,
      network_online: navigator.onLine,
    });
    if (event.code === WS_AUTH_REQUIRED_CLOSE_CODE) {
      logoutProductSession();
      return;
    }
    void (async () => {
      const handshake = await probeProductAuth();
      if (productSessionAbandoned || socket !== undefined) return;
      if (handshake === "logout") {
        logoutProductSession();
        return;
      }
      // 200 reconnects; 5xx / network / 404 keep the cookie and retry.
      scheduleReconnect(conn.connectionLost());
    })();
  };
  ws.onerror = (): void => {
    // Closing a superseded socket can emit an error after its replacement has
    // already connected. It no longer owns global state and is not an outage.
    if (socket !== ws) return;
    reportClientLog("error", "websocket_error", "Cowboy WebSocket failed", {
      reason: connectReason,
      attempt: reconnecting ? reconnectAttempts : 0,
      ready_state: ws.readyState,
      network_online: navigator.onLine,
    });
    if (socket === ws) ws.close();
  };
}

/** Returns whether the command actually went out (socket OPEN). Durable queue
 *  mutations remain pending on `false`; ephemeral transcript sends fail. */
export function send(cmd: Inbound): boolean {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(cmd));
    return true;
  }
  return false;
}

function isConnected(): boolean {
  return socket?.readyState === WebSocket.OPEN;
}

// --- Optimistic sends (local, never synced) ---------------------------------
// Show a staged/sent item INSTANTLY, reconciled away by cmid when the daemon
// echoes it. Drafts + queue optimism lives in the per-session queue sync clients
// (`qClient`); chat sends (submit-when-idle) in the `optimisticMessages` overlay
// (the transcript isn't a small-value sync state). Both share the timer/status
// machinery below.

/** How long an optimistic row waits before showing the gradient shimmer. Under
 *  this, it renders as a normal row — a fast LAN/tailnet confirm (<~100ms)
 *  replaces it before the shimmer ever appears, so a quick send never flashes a
 *  loader. ~100ms is the human "instant" threshold (Nielsen/RAIL); 200ms leaves
 *  margin for jitter so the fast path is never a flicker. */
const SHIMMER_DELAY_MS = 200;
/** No daemon echo by here → treat the send as failed (WS dropped mid-flight). */
const SEND_TIMEOUT_MS = 10_000;

function newCmid(): string {
  return `c-${newUuid()}`;
}

// --- Optimistic sync (Cowboy state-sync component) --------------------------
// The daemon is the arbiter for each synced `state`. Each client applies a
// mutation LOCALLY for an instant change, sends a generic `sync` command, and
// folds the arbiter's `sync_patch` back (drops confirmed pending, converges on
// server order). Synced states today: "title" (session_id→title map) and
// "order" (the session-id ordering) — both overlaid onto the session list by
// `deriveSessions`, so the rest of the UI is untouched. The bespoke
// rename/reorder commands are retired for the web client.

// The daemon broadcasts the full session list (`sessions`); the synced states
// are overlays on top, so we keep the raw list and re-derive on any change.
let rawSessions: SessionMeta[] = [];

type TitleMap = Readonly<Record<string, string>>;
type OrderList = readonly string[];

const titleMutators = {
  rename: (m: TitleMap, a: { session_id: string; title: string }): TitleMap => ({ ...m, [a.session_id]: a.title }),
} satisfies Mutators<TitleMap>;
const orderMutators = {
  // Absolute set: the arbiter's reorder is a full new ordering.
  reorder: (_m: OrderList, a: { order: OrderList }): OrderList => a.order,
} satisfies Mutators<OrderList>;

interface SyncEntry {
  applyPatch: (version: number, value: unknown, confirmed: string[], resync: boolean) => void;
  resend: () => void;
  hydrate: () => Promise<void>;
  flush: () => Promise<void>;
}
const syncClients = new Map<string, SyncEntry>();
const syncBase = newCmid(); // namespaces mutation ids across states + this tab

/** Wire one synced state to the generic channel via the shared op-based tier
 *  (`replicatedStore`): instant optimistic mutate (auto-sent), patch fold,
 *  resend-on-reconnect, and an IndexedDB durable outbox. `onChange` = re-derive
 *  the session list, so mutate / patch / hydrate all re-render for free. */
function registerSync<T, M extends Mutators<T>>(
  syncState: string,
  mutators: M,
  initial: T,
  onChange: () => void = commitSessions,
): { view: () => T; mutate: <K extends keyof M & string>(name: K, args: ArgsOf<T, M, K>) => void } {
  const store = replicatedStore<T, M>({
    clientId: `${syncBase}:${syncState}`,
    mutators,
    initial,
    send: (m): void => {
      send({ type: "sync", state: syncState, id: m.id, name: m.name, args: m.args });
    },
    onChange,
    // Instant-load + durable outbox: cache {base, pending} to IndexedDB. On
    // reload we hydrate this BEFORE the socket opens (see connect()), so the
    // last-known title/order paint immediately; the first server patch arrives
    // as a forced resync and overwrites stale base, while any unconfirmed
    // mutation re-sends. The key is NOT tab-namespaced (no syncBase) so every
    // tab shares one cache — they all sync to the same server truth anyway.
    local: idbPersistence<ClientSnapshot<T>>(`cowboy:sync:${syncState}`),
  });
  syncClients.set(syncState, {
    applyPatch: (version, value, confirmed, resync): void => {
      store.applyPatch(snapshotPatch(version, value as T, confirmed), { force: resync });
    },
    resend: (): void => {
      store.resend();
    },
    hydrate: (): Promise<void> => store.hydrate(),
    flush: (): Promise<void> => store.flush(),
  });
  return {
    view: (): T => store.get(),
    mutate: (name, args): void => {
      store.mutate(name, args);
    },
  };
}

const titleSync = registerSync<TitleMap, typeof titleMutators>("title", titleMutators, {});
const orderSync = registerSync<OrderList, typeof orderMutators>("order", orderMutators, []);

export interface MobileReviewTabState {
  readonly path: string;
  readonly pinned: boolean;
}

export interface MobileReviewState {
  readonly mode: "files" | "git";
  readonly tabs: readonly MobileReviewTabState[];
  readonly active?: string;
  readonly progress: Readonly<Record<string, string>>;
  readonly positions: Readonly<
    Record<string, { readonly line: number; readonly revision?: string }>
  >;
}

const EMPTY_MOBILE_REVIEW_STATE: MobileReviewState = {
  mode: "git",
  tabs: [],
  progress: {},
  positions: {},
};

const mobileReviewMutators = {
  open: (value: MobileReviewState, args: { path: string }): MobileReviewState => {
    const existing = value.tabs.find((tab) => tab.path === args.path);
    let tabs = existing ? [...value.tabs] : [...value.tabs, { path: args.path, pinned: false }];
    if (tabs.length > 12) {
      const evict = tabs.findIndex((tab) => !tab.pinned);
      tabs = tabs.filter((_, index) => index !== (evict < 0 ? 0 : evict));
    }
    return { ...value, mode: "files", tabs, active: args.path };
  },
  close: (value: MobileReviewState, args: { path: string }): MobileReviewState => {
    const tabs = value.tabs.filter((tab) => tab.path !== args.path);
    const active = value.active === args.path ? tabs.at(-1)?.path : value.active;
    return {
      mode: value.mode,
      tabs,
      progress: value.progress,
      positions: value.positions,
      ...(active === undefined ? {} : { active }),
    };
  },
  reorder: (value: MobileReviewState, args: { paths: readonly string[] }): MobileReviewState => {
    const position = new Map(args.paths.map((path, index) => [path, index]));
    return {
      ...value,
      tabs: [...value.tabs].sort((left, right) =>
        (position.get(left.path) ?? Number.MAX_SAFE_INTEGER) -
        (position.get(right.path) ?? Number.MAX_SAFE_INTEGER)
      ),
    };
  },
  setPinned: (
    value: MobileReviewState,
    args: { path: string; pinned: boolean },
  ): MobileReviewState => ({
    ...value,
    tabs: value.tabs.map((tab) =>
      tab.path === args.path ? { ...tab, pinned: args.pinned } : tab
    ),
  }),
  activate: (
    value: MobileReviewState,
    args: { path: string | null },
  ): MobileReviewState => ({
    mode: value.mode,
    tabs: value.tabs,
    progress: value.progress,
    positions: value.positions,
    ...(args.path && value.tabs.some((tab) => tab.path === args.path)
      ? { active: args.path }
      : {}),
  }),
  setMode: (
    value: MobileReviewState,
    args: { mode: "files" | "git" },
  ): MobileReviewState => ({ ...value, mode: args.mode }),
  markReviewed: (
    value: MobileReviewState,
    args: { key: string; revision: string | null },
  ): MobileReviewState => {
    const progress = { ...value.progress };
    if (args.revision === null) delete progress[args.key];
    else progress[args.key] = args.revision;
    return { ...value, progress };
  },
  setPosition: (
    value: MobileReviewState,
    args: { path: string; line: number; revision: string | null },
  ): MobileReviewState => ({
    ...value,
    positions: {
      ...value.positions,
      [args.path]: {
        line: Math.max(1, Math.trunc(args.line)),
        ...(args.revision === null ? {} : { revision: args.revision }),
      },
    },
  }),
} satisfies Mutators<MobileReviewState>;

type MobileReviewMutation = keyof typeof mobileReviewMutators & string;
const mobileReviewClients = new Map<
  string,
  ReturnType<typeof registerSync<MobileReviewState, typeof mobileReviewMutators>>
>();

function mobileReviewClient(sessionId: string) {
  let client = mobileReviewClients.get(sessionId);
  if (!client) {
    client = registerSync(
      `mobile-review:${sessionId}`,
      mobileReviewMutators,
      EMPTY_MOBILE_REVIEW_STATE,
      () => commitMobileReview(sessionId),
    );
    mobileReviewClients.set(sessionId, client);
  }
  return client;
}

function commitMobileReview(sessionId: string): void {
  const current = mobileReviewClient(sessionId).view();
  // Older IndexedDB snapshots predate per-file positions. Normalize at the
  // boundary so an offline Mobile client can upgrade without clearing state.
  const value: MobileReviewState = {
    ...current,
    positions: current.positions ?? {},
  };
  setState({
    ...state,
    mobileReviewStates: { ...state.mobileReviewStates, [sessionId]: value },
  });
}

export function useMobileReviewState(sessionId: string | undefined): MobileReviewState {
  return useStoreSelector((snapshot) =>
    sessionId
      ? snapshot.mobileReviewStates[sessionId] ?? EMPTY_MOBILE_REVIEW_STATE
      : EMPTY_MOBILE_REVIEW_STATE
  );
}

export function mutateMobileReview<K extends MobileReviewMutation>(
  sessionId: string,
  name: K,
  args: ArgsOf<MobileReviewState, typeof mobileReviewMutators, K>,
): void {
  // Registered project checkouts are read-only code contexts, not agent
  // sessions. Their tabs and navigation remain local to the mounted Review
  // surface; sending them through the session sync channel produces an
  // `unknown mobile review session` warning and can never persist server-side.
  if (sessionId.startsWith("workspace::")) {
    const current = state.mobileReviewStates[sessionId] ?? EMPTY_MOBILE_REVIEW_STATE;
    // `name` and `args` are linked by K at this public boundary. TypeScript
    // widens an indexed generic function to the intersection of every argument
    // shape, so narrow only the internal call after that relationship is proven.
    const mutate = mobileReviewMutators[name] as unknown as (
      value: MobileReviewState,
      args: unknown,
    ) => MobileReviewState;
    const value = mutate(current, args);
    setState({
      ...state,
      mobileReviewStates: { ...state.mobileReviewStates, [sessionId]: value },
    });
    return;
  }
  mobileReviewClient(sessionId).mutate(name, args);
  commitMobileReview(sessionId);
}

/** Apply the title + order overlays to the raw session list (title override
 *  wins; then a stable sort by the synced order — ids not in `order` keep their
 *  relative position at the end, mirroring the daemon's `sort_by_id_order`). */
function deriveSessions(raw: SessionMeta[], titles: TitleMap, order: OrderList): SessionMeta[] {
  const titled = raw.map((s) => {
    const t = titles[s.id];
    return t !== undefined && t !== s.title ? { ...s, title: t } : s;
  });
  if (order.length === 0) return titled;
  const pos = new Map(order.map((id, i): [string, number] => [id, i]));
  return titled
    .map((s, i): { s: SessionMeta; i: number } => ({ s, i }))
    .sort((a, b) => {
      const pa = pos.get(a.s.id) ?? Number.MAX_SAFE_INTEGER;
      const pb = pos.get(b.s.id) ?? Number.MAX_SAFE_INTEGER;
      return pa !== pb ? pa - pb : a.i - b.i;
    })
    .map((x) => x.s);
}

/** Re-derive the rendered session list from the raw list + every synced overlay
 *  and commit it. Called on a `sessions` broadcast and on any sync patch/mutate. */
function commitSessions(): void {
  const titles = titleSync.view();
  setState({ ...state, sessions: deriveSessions(rawSessions, titles, orderSync.view()), titleOverrides: titles });
}

/** Optimistic rename via the title-sync engine: instant local + send; re-sent on
 *  reconnect (id ⇒ idempotent). Empty title is a no-op (also rejected server). */
export function renameSession(sessionId: string, title: string): void {
  const trimmed = title.trim();
  if (trimmed === "") return;
  titleSync.mutate("rename", { session_id: sessionId, title: trimmed });
}

/** Whether the live WS is currently open. False the moment it drops (onclose),
 *  true on (re)open — drives the turn-status pill's "Reconnecting…" state. */
export function useConnected(): boolean {
  return useStoreSelector((snapshot) => snapshot.connected);
}

// --- Queue + drafts optimistic sync (per session) ----------------------------
// Each session's queue+drafts is a sync state "queue:<sid>". The daemon is the
// arbiter; the client applies ADDs and local-first moves instantly, then
// rebases onto the latest server list and reconciles by cmid / source id.
// pending/sending/failed STATUS is local presentation, kept in `qStatus`
// (keyed by cmid) and merged at commit — never in the synced value.

type QValue = QueueValue<QueuedMessage>;
const qMut = {
  addDraft: (v: QValue, a: { row: QueuedMessage }): QValue => queueMutators.addDraft(v, a),
  scheduleDraft: (v: QValue, a: { row: QueuedMessage }): QValue =>
    queueMutators.scheduleDraft(v, a),
  rescheduleDraft: (v: QValue, a: { id: string; row: QueuedMessage }): QValue =>
    queueMutators.rescheduleDraft(v, a),
  editDraft: (v: QValue, a: { id: string; row: QueuedMessage }): QValue =>
    queueMutators.editDraft(v, a),
  editQueue: (v: QValue, a: { id: string; row: QueuedMessage }): QValue =>
    queueMutators.editQueue(v, a),
  removeDraft: (v: QValue, a: { id: string }): QValue =>
    queueMutators.removeDraft(v, a),
  removeQueue: (v: QValue, a: { id: string }): QValue =>
    queueMutators.removeQueue(v, a),
  unscheduleDraft: (v: QValue, a: { id: string; row: QueuedMessage }): QValue =>
    queueMutators.unscheduleDraft(v, a),
  addQueue: (v: QValue, a: { row: QueuedMessage }): QValue => queueMutators.addQueue(v, a),
  submitPrompt: (v: QValue, a: { row: QueuedMessage }): QValue =>
    queueMutators.submitPrompt(v, a),
  forceQueue: (v: QValue, a: { row: QueuedMessage }): QValue => queueMutators.forceQueue(v, a),
  frontQueue: (v: QValue, a: { row: QueuedMessage }): QValue => queueMutators.frontQueue(v, a),
  activateDraft: (v: QValue, a: { id: string; row?: QueuedMessage }): QValue =>
    queueMutators.activateDraft(v, a),
  sendQueued: (v: QValue, a: { id: string; row: QueuedMessage }): QValue =>
    queueMutators.sendQueued(v, a),
  forceQueued: (v: QValue, a: { id: string; row: QueuedMessage }): QValue =>
    queueMutators.forceQueued(v, a),
  returnQueuedToDraft: (v: QValue, a: { id: string; row: QueuedMessage }): QValue =>
    queueMutators.returnQueuedToDraft(v, a),
} satisfies Mutators<QValue>;
const qClients = new Map<string, ReplicatedStore<QValue, typeof qMut>>();
const qStatus = new Map<string, "pending" | "sending" | "failed">();
const SILENT_QUEUE_MUTATORS = new Set([
  "editDraft",
  "editQueue",
  "rescheduleDraft",
  "removeDraft",
  "removeQueue",
  "unscheduleDraft",
]);
// inFlight rows whose transcript bubble was already reconciled by a user-echo.
// Stops commitQueue from resurrecting the same bubble before the queue patch
// confirms the hide mutation.
const suppressedInFlight = new Set<string>();

function commandForQueueMutation(sessionId: string, m: { name: string; id: string; args: unknown }): Inbound {
  const args = m.args as { id?: string; row?: QueuedMessage };
  if (m.name === "addDraft" && args.row !== undefined) {
    return {
      type: "add_draft",
      session_id: sessionId,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
      cmid: m.id,
    };
  }
  if (m.name === "scheduleDraft" && args.row?.schedule !== undefined) {
    return {
      type: "schedule_draft",
      session_id: sessionId,
      cmid: m.id,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
      fire_at_ms: args.row.schedule.fire_at_ms,
      delivery: args.row.schedule.delivery,
    };
  }
  if (
    m.name === "rescheduleDraft" &&
    args.id !== undefined &&
    args.row?.schedule !== undefined
  ) {
    return {
      type: "schedule_draft",
      session_id: sessionId,
      id: args.id,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
      fire_at_ms: args.row.schedule.fire_at_ms,
      delivery: args.row.schedule.delivery,
    };
  }
  if (
    (m.name === "editDraft" || m.name === "editQueue") &&
    args.id !== undefined &&
    args.row !== undefined
  ) {
    return {
      type: m.name === "editDraft" ? "edit_draft" : "edit_queued",
      session_id: sessionId,
      id: args.id,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
    };
  }
  if (m.name === "removeDraft" && args.id !== undefined) {
    return { type: "remove_draft", session_id: sessionId, id: args.id };
  }
  if (m.name === "removeQueue" && args.id !== undefined) {
    return { type: "remove_queued", session_id: sessionId, id: args.id };
  }
  if (m.name === "unscheduleDraft" && args.id !== undefined) {
    return { type: "unschedule_draft", session_id: sessionId, id: args.id };
  }
  if (m.name === "activateDraft" && args.id !== undefined) {
    return { type: "activate_draft", session_id: sessionId, id: args.id };
  }
  if (m.name === "sendQueued" && args.id !== undefined) {
    return { type: "request_send_queued", session_id: sessionId, id: args.id };
  }
  if (m.name === "forceQueued" && args.id !== undefined) {
    return { type: "force_push_queued", session_id: sessionId, id: args.id };
  }
  if (m.name === "returnQueuedToDraft" && args.id !== undefined) {
    return { type: "queued_to_draft", session_id: sessionId, id: args.id };
  }
  const row = args.row;
  if (row === undefined) {
    return { type: "activate_draft", session_id: sessionId, id: args.id ?? m.id };
  }
  return {
    type: "submit",
    session_id: sessionId,
    text: row.text,
    content: contentOf(row.text, row.attachments),
    cmid: m.id,
    ...(m.name === "forceQueue" && { force: true }),
    ...(m.name === "frontQueue" && { front: true }),
  };
}

function qClient(sessionId: string): ReplicatedStore<QValue, typeof qMut> {
  let c = qClients.get(sessionId);
  if (c === undefined) {
    c = replicatedStore<QValue, typeof qMut>({
      clientId: `${syncBase}:queue:${sessionId}`,
      mutators: qMut,
      initial: emptyQueueValue<QueuedMessage>(),
      // Adds send add_draft/submit (cmid = mutation id). Local-first moves send
      // the matching activate/request_send/force/queued_to_draft command. Used
      // both for the initial send (via mutate) and the reconnect resend.
      send: (m): void => {
        const sent = send(commandForQueueMutation(sessionId, m));
        if (SILENT_QUEUE_MUTATORS.has(m.name)) return;
        const attempt = QUEUE_TRANSITION_MUTATORS.has(m.name)
          ? {
            status: statusAfterExplicitSend(sent),
            armConfirmationTimeout: sent,
          }
          : durableDeliveryAttempt(sent);
        qStatus.set(m.id, attempt.status);
        const echoCmid = (m.args as { row?: QueuedMessage }).row?.cmid;
        if (echoCmid !== undefined && echoCmid !== m.id) {
          qStatus.set(echoCmid, attempt.status);
        }
        if (attempt.armConfirmationTimeout) armQTimers(sessionId, m.id);
      },
      onChange: (): void => {
        commitQueue(sessionId);
      },
      // Durable outbox: cache {base, pending} per session. On reload we enumerate
      // these keys and hydrate each qClient BEFORE the socket opens (see
      // connect()), so a staged/queued message painted instantly survives the
      // reload and re-sends; the per-session `queue_resync` (force) that follows
      // is the authority that corrects any stale cached base.
      local: idbPersistence<ClientSnapshot<QValue>>(
        `cowboy:sync:queue:${sessionId}`,
        { strictWrites: true },
      ),
    });
    qClients.set(sessionId, c);
  }
  return c;
}

/** Eager-restore every per-session queue durable outbox cached in IndexedDB,
 *  BEFORE the socket opens (called from connect's pre-open hydrate). Enumerating
 *  the keys is what makes this correct: the qClients are created lazily during
 *  patch handling, so without enumerating we couldn't know which sessions to
 *  hydrate ahead of the resync. Each hydrate restores {base, pending} + renders
 *  via onChange; the queue_resync (force) on connect then corrects stale base. */
async function hydrateCachedQueues(): Promise<void> {
  const prefix = "cowboy:sync:queue:";
  const keys = await idbListKeys();
  const outcomes = await Promise.allSettled(
    keys.filter((k) => k.startsWith(prefix)).map((k) => qClient(k.slice(prefix.length)).hydrate()),
  );
  for (const outcome of outcomes) {
    if (outcome.status === "rejected") {
      console.warn("queue outbox hydrate failed", outcome.reason);
    }
  }
}

/** Render queue + drafts for one session: the sync client's view (server base +
 *  rebased optimistic adds), with the local `status` merged onto each still-
 *  pending optimistic row. */
function commitQueue(sessionId: string): void {
  const c = qClients.get(sessionId);
  const view: QValue = c ? c.get() : emptyQueueValue<QueuedMessage>();
  const pend = new Set((c?.pending() ?? []).map((m) => m.id));
  const withStatus = (rows: readonly QueuedMessage[]): QueuedMessage[] =>
    rows.map((r) => {
      if (r.cmid === undefined) return r;
      const status = qStatus.get(r.cmid) ?? (pend.has(r.cmid) ? "pending" : undefined);
      return status !== undefined ? { ...r, status } : r;
    });
  const queues = new Map(state.queues);
  const drafts = new Map(state.drafts);
  const q = withStatus(view.queue);
  const d = withStatus(view.drafts);
  if (q.length > 0) queues.set(sessionId, q);
  else queues.delete(sessionId);
  if (d.length > 0) drafts.set(sessionId, d);
  else drafts.delete(sessionId);
  const inFlightCmids = new Set(
    (view.inFlight ?? []).flatMap((row) => row.cmid === undefined ? [] : [row.cmid]),
  );
  const existing = state.optimisticMessages.get(sessionId) ?? [];
  const kept = existing.filter((message) =>
    message.cmid === undefined ||
    (!suppressedInFlight.has(message.cmid) && !inFlightCmids.has(message.cmid))
  );
  const bubbles = [...kept];
  for (const row of view.inFlight ?? []) {
    if (row.cmid !== undefined && suppressedInFlight.has(row.cmid)) continue;
    if (row.cmid !== undefined && bubbles.some((message) => message.cmid === row.cmid)) {
      continue;
    }
    bubbles.push({
      ...row,
      status: (row.cmid !== undefined ? qStatus.get(row.cmid) : undefined) ?? row.status ?? "pending",
    });
  }
  const optimisticMessages = new Map(state.optimisticMessages);
  if (bubbles.length > 0) optimisticMessages.set(sessionId, bubbles);
  else optimisticMessages.delete(sessionId);
  setState({ ...state, queues, drafts, optimisticMessages });
}

/** Arm pending→sending (shimmer) and →failed timers for an optimistic queue row,
 *  keyed by cmid; flips `qStatus` and re-renders. */
function armQTimers(sessionId: string, cmid: string): void {
  clearOptTimers(cmid);
  optTimers.set(cmid, {
    shimmer: setTimeout(() => {
      if (qStatus.get(cmid) === "pending") {
        qStatus.set(cmid, "sending");
        commitQueue(sessionId);
      }
    }, SHIMMER_DELAY_MS),
    fail: setTimeout(() => {
      if (qStatus.get(cmid) !== "failed") {
        qStatus.set(cmid, "failed");
        commitQueue(sessionId);
      }
      clearOptTimers(cmid);
    }, SEND_TIMEOUT_MS),
  });
}

/** Optimistic add to drafts or queue: mutate (id = cmid, so the server echo
 *  drops exactly this row — no ghost) + send the command + arm status timers. */
async function qAdd(
  target: "drafts" | "queue" | "transcript" | "scheduled",
  sessionId: string,
  text: string,
  attachments: Attachment[],
  opts: {
    mode?: "back" | "front" | "force";
    origin?: DeliveryOrigin;
    schedule?: DraftSchedule;
    cmid?: string;
  } = {},
): Promise<void> {
  const mode = opts.mode ?? "back";
  const origin = opts.origin ?? "composer";
  const cmid = opts.cmid ?? newCmid();
  const row: QueuedMessage = {
    id: `opt-${cmid}`,
    text,
    attachments,
    cmid,
    origin,
    ...(opts.schedule !== undefined ? { schedule: opts.schedule } : {}),
  };
  const store = qClient(sessionId);
  // Set status BEFORE mutating: the local apply fires `onChange` → commitQueue,
  // which reads this status. `mutateDurably` commits the outbox transaction before
  // its transport callback can send. The source editor may clear only after this
  // barrier resolves.
  qStatus.set(cmid, "pending");
  const mutator = target === "drafts"
    ? "addDraft"
    : target === "scheduled"
    ? "scheduleDraft"
    : target === "transcript"
    ? "submitPrompt"
    : mode === "force"
    ? "forceQueue"
    : mode === "front"
    ? "frontQueue"
    : "addQueue";
  try {
    await store.mutateDurably(mutator, { row }, cmid);
  } catch (error) {
    qStatus.delete(cmid);
    commitQueue(sessionId);
    throw error;
  }
  if (target !== "transcript") {
    revealPendingArrival({
      kind: target === "drafts" || target === "scheduled" ? "draft" : "queued",
      id: row.id,
      cmid,
    });
  }
  await waitForState(
    (snapshot) =>
      target === "transcript"
        ? (snapshot.optimisticMessages.get(sessionId) ?? []).some((message) =>
          message.cmid === cmid
        )
        : (target === "drafts" || target === "scheduled"
          ? snapshot.drafts
          : snapshot.queues)
          .get(sessionId)
          ?.some((message) => message.cmid === cmid) === true,
    target === "drafts" || target === "scheduled" ? "Save draft" : "Send message",
  );
}

/** Retry a failed optimistic queue/draft row from THIS device: re-anchor it to
 *  the END (bump) then resend (same cmid ⇒ idempotent). */
export function retryQueued(sessionId: string, cmid: string): void {
  const c = qClients.get(sessionId);
  if (c === undefined) return;
  const pending = c.pending().find((mutation) => mutation.id === cmid);
  if (pending !== undefined) {
    c.bump(cmid);
    const attempt = retryDeliveryAttempt(send(commandForQueueMutation(sessionId, pending)));
    qStatus.set(cmid, attempt.status);
    const echoCmid = (pending.args as { row?: QueuedMessage }).row?.cmid;
    if (echoCmid !== undefined && echoCmid !== cmid) qStatus.set(echoCmid, attempt.status);
    commitQueue(sessionId);
    if (attempt.armConfirmationTimeout) armQTimers(sessionId, cmid);
    return;
  }
  const view = c.get();
  const inDrafts = view.drafts.some((r) => r.cmid === cmid);
  const row = (inDrafts ? view.drafts : view.queue).find((r) => r.cmid === cmid);
  if (row === undefined) return;
  const attempt = retryDeliveryAttempt(send(
    inDrafts
      ? { type: "add_draft", session_id: sessionId, text: row.text, content: contentOf(row.text, row.attachments), cmid }
      : { type: "submit", session_id: sessionId, text: row.text, content: contentOf(row.text, row.attachments), cmid },
  ));
  qStatus.set(cmid, attempt.status);
  commitQueue(sessionId);
  if (attempt.armConfirmationTimeout) armQTimers(sessionId, cmid);
}

/** Discard a (failed) optimistic queue/draft row locally — it never reached the
 *  daemon, so nothing server-side to remove. Persist the outbox removal before
 *  reporting success so a reload cannot resurrect and resend a discarded row. */
export async function discardQueued(sessionId: string, cmid: string): Promise<void> {
  await qClients.get(sessionId)?.confirmDurably([cmid]);
  clearOptTimers(cmid);
  qStatus.delete(cmid);
  commitQueue(sessionId);
}

function pendingNamed(
  sessionId: string,
  cmid: string,
): { id: string; name: string; args: unknown } | undefined {
  return qClients.get(sessionId)?.pending().find((mutation) => {
    const echoCmid = (mutation.args as { row?: QueuedMessage }).row?.cmid;
    return mutation.id === cmid || echoCmid === cmid;
  });
}

/** Park a failed local send back in the list it left. */
export async function returnFailedQueued(sessionId: string, cmid: string): Promise<void> {
  const pending = pendingNamed(sessionId, cmid);
  if (pending !== undefined && QUEUE_TRANSITION_MUTATORS.has(pending.name)) {
    await qClients.get(sessionId)?.confirmDurably([pending.id]);
    clearOptTimers(pending.id);
    qStatus.delete(pending.id);
    const echoCmid = (pending.args as { row?: QueuedMessage }).row?.cmid;
    if (echoCmid !== undefined) {
      qStatus.delete(echoCmid);
      suppressedInFlight.add(echoCmid);
      patchMessage(sessionId, echoCmid, "drop");
    }
    commitQueue(sessionId);
    return;
  }
  const view = qClients.get(sessionId)?.get();
  const inDrafts = view?.drafts.some((row) => row.cmid === cmid) === true;
  const row = (inDrafts ? view?.drafts : view?.queue)?.find((item) => item.cmid === cmid);
  if (row === undefined) return;
  const home = homeForOrigin(row.origin ?? "composer");
  // Persist the replacement first. If IndexedDB rejects the write, retain the
  // original failed row as the recovery source instead of clearing it first.
  if (home === "draft" && !inDrafts) {
    await qAdd("drafts", sessionId, row.text, row.attachments, { origin: "composer" });
  } else if (home === "queue" && inDrafts) {
    await qAdd("queue", sessionId, row.text, row.attachments, { origin: "queue" });
  }
  await discardQueued(sessionId, cmid);
}

export async function returnFailedMessage(sessionId: string, cmid: string): Promise<void> {
  const row = state.optimisticMessages.get(sessionId)?.find((message) => message.cmid === cmid);
  if (row === undefined) return;
  const pending = pendingNamed(sessionId, cmid);
  if (pending !== undefined && QUEUE_TRANSITION_MUTATORS.has(pending.name)) {
    await returnFailedQueued(sessionId, pending.id);
    return;
  }
  const home = homeForOrigin(row.origin ?? "composer");
  await qAdd(home === "queue" ? "queue" : "drafts", sessionId, row.text, row.attachments, {
    origin: row.origin ?? "composer",
  });
  await discardMessage(sessionId, cmid);
}

/** Fold a queue `sync_patch` for one session: apply (force on resync), clear the
 *  status/timers of any cmid the server confirmed, and cross-signal the chat
 *  overlay (a queue cmid landing also drops a wrong-guessed optimistic bubble). */
function applyQueuePatch(sessionId: string, version: number, value: unknown, confirmed: string[], resync: boolean): void {
  const wire = value as { queue?: WireQueued[]; drafts?: WireQueued[] };
  const next: QValue = {
    queue: fromWire(wire.queue ?? []),
    drafts: fromWire(wire.drafts ?? []),
    inFlight: [],
  };
  const store = qClient(sessionId);
  const settled = settledTransitionIds(
    store.pending(),
    next,
    sameQueuedContent,
  );
  const settledSuppressed: string[] = [];
  if (settled.length > 0) {
    for (const id of settled) {
      const pending = store.pending().find((mutation) => mutation.id === id);
      if (pending !== undefined && QUEUE_TRANSITION_MUTATORS.has(pending.name)) {
        const echoCmid = (pending.args as { row?: QueuedMessage }).row?.cmid ?? id;
        suppressedInFlight.add(echoCmid);
        settledSuppressed.push(echoCmid);
      }
      clearOptTimers(id);
      qStatus.delete(id);
    }
    store.confirm(settled);
  }
  store.applyPatch(snapshotPatch(version, next, confirmed), { force: resync });
  const orphaned = claimOrphanedPendingEdits(
    sessionId,
    new Set(next.queue.map((row) => row.id)),
    new Set(next.drafts.map((row) => row.id)),
  );
  for (const record of orphaned) {
    const alreadyRecovered = next.drafts.some((row) =>
      row.cmid === record.recoveryCmid
    ) || store.pending().some((mutation) => mutation.id === record.recoveryCmid);
    if (alreadyRecovered) {
      finishOrphanedPendingEdit(record, true);
      continue;
    }
    if (!record.text.trim() && record.attachments.length === 0) {
      finishOrphanedPendingEdit(record, false);
      continue;
    }
    void qAdd("drafts", sessionId, record.text, record.attachments, {
      origin: "composer",
      cmid: record.recoveryCmid,
    }).then(() => finishOrphanedPendingEdit(record, true)).catch(() => {
      finishOrphanedPendingEdit(record, false);
    });
  }
  for (const cmid of confirmed) {
    if (qStatus.has(cmid)) {
      clearOptTimers(cmid);
      qStatus.delete(cmid);
    }
    suppressedInFlight.delete(cmid);
  }
  for (const cmid of settledSuppressed) suppressedInFlight.delete(cmid);
  if (confirmed.length > 0) {
    const set = new Set<string | undefined>(confirmed);
    setState({ ...state, optimisticMessages: reconcileOptimistic(state.optimisticMessages, sessionId, set) });
  }
  commitQueue(sessionId);
}

// The optimistic CHAT-bubble overlay is projected from the same durable queue
// sync state as drafts and queued prompts. It is keyed by cmid and reconciled
// by the user-echo Envelope (or a wrong-guess queue patch).

// cmid → its pending/timeout timers, so reconcile/retry can clear them. Shared
// by the chat overlay AND the queue path (`armQTimers`).
const optTimers = new Map<string, { shimmer?: ReturnType<typeof setTimeout>; fail?: ReturnType<typeof setTimeout> }>();
function clearOptTimers(cmid: string): void {
  const t = optTimers.get(cmid);
  if (t?.shimmer) clearTimeout(t.shimmer);
  if (t?.fail) clearTimeout(t.fail);
  optTimers.delete(cmid);
}

/** Mutate one optimistic chat bubble's status in place (immutably), or drop it. */
function patchMessage(sessionId: string, cmid: string, patch: ((m: QueuedMessage) => QueuedMessage) | "drop"): void {
  const list = state.optimisticMessages.get(sessionId);
  if (!list) return;
  const next = patch === "drop"
    ? list.filter((m) => m.cmid !== cmid)
    : list.map((m) => (m.cmid === cmid ? patch(m) : m));
  const map = new Map(state.optimisticMessages);
  if (next.length > 0) map.set(sessionId, next);
  else map.delete(sessionId);
  setState({ ...state, optimisticMessages: map });
}

/** Arm the pending→sending (shimmer) and →failed timers for an optimistic
 *  chat bubble. */
function armMsgTimers(sessionId: string, cmid: string): void {
  clearOptTimers(cmid);
  optTimers.set(cmid, {
    shimmer: setTimeout(() => {
      patchMessage(sessionId, cmid, (m) => (m.status === "pending" ? { ...m, status: "sending" } : m));
    }, SHIMMER_DELAY_MS),
    fail: setTimeout(() => {
      patchMessage(sessionId, cmid, (m) => (m.status === "failed" ? m : { ...m, status: "failed" }));
      clearOptTimers(cmid);
    }, SEND_TIMEOUT_MS),
  });
}

/** Drop confirmed optimistic rows (cmid now in the server set) + clear their
 *  timers. Pure: returns the next map (or the same ref if nothing changed). */
function reconcileOptimistic(
  current: Map<string, QueuedMessage[]>,
  sessionId: string,
  serverCmids: Set<string | undefined>,
): Map<string, QueuedMessage[]> {
  const list = current.get(sessionId);
  if (!list) return current;
  const kept = list.filter((m) => {
    if (m.cmid !== undefined && serverCmids.has(m.cmid)) {
      clearOptTimers(m.cmid);
      suppressedInFlight.add(m.cmid);
      return false;
    }
    return true;
  });
  if (kept.length === list.length) return current;
  const next = new Map(current);
  if (kept.length > 0) next.set(sessionId, kept);
  else next.delete(sessionId);
  return next;
}

/** Retry a failed optimistic chat bubble from THIS device (idempotent — the
 *  daemon dedupes by cmid). Back to `pending`, re-armed. */
export function retryMessage(sessionId: string, cmid: string): void {
  const row = state.optimisticMessages.get(sessionId)?.find((m) => m.cmid === cmid);
  if (!row) return;
  const pending = qClients.get(sessionId)?.pending().find((mutation) => {
    const echoCmid = (mutation.args as { row?: QueuedMessage }).row?.cmid;
    return mutation.id === cmid || echoCmid === cmid;
  });
  if (pending !== undefined) {
    retryQueued(sessionId, pending.id);
    return;
  }
  const attempt = retryDeliveryAttempt(send({
    type: "submit",
    session_id: sessionId,
    text: row.text,
    content: contentOf(row.text, row.attachments),
    cmid,
  }));
  patchMessage(sessionId, cmid, (m) => ({ ...m, status: attempt.status }));
  if (attempt.armConfirmationTimeout) armMsgTimers(sessionId, cmid);
}

/** Discard a (usually failed) optimistic chat bubble locally — it never reached
 *  the daemon, so there's nothing server-side to remove. */
export async function discardMessage(sessionId: string, cmid: string): Promise<void> {
  const pending = pendingNamed(sessionId, cmid);
  if (pending !== undefined) {
    await discardQueued(sessionId, pending.id);
    return;
  }
  clearOptTimers(cmid);
  patchMessage(sessionId, cmid, "drop");
}

/** Optimistic chat send (submit-when-idle): persist first, show a transcript
 *  bubble, then submit. A closed socket leaves it pending for reconnect. */
function optimisticMessage(
  sessionId: string,
  text: string,
  attachments: Attachment[],
  origin: DeliveryOrigin = "composer",
): Promise<void> {
  return qAdd("transcript", sessionId, text, attachments, { origin });
}

// Tell the daemon the user opened/selected `id` so it revives that session's
// agent before the user types — design §7 "revive on open". Remembered so the
// id is re-asserted on every reconnect (see connect's onopen). Cheap + a
// server-side no-op when the agent is already alive, so it's fine to call on
// every navigation.
export function openSession(id: string): void {
  openedSessionId = id;
  touchTranscriptSession(id);
  send({ type: "open_session", session_id: id });
  void hydrateSession(id);
}

// Mark a session hydrated WITHOUT waiting for a server snapshot — called the
// moment the client creates a new session. A brand-new session has no history
// and the agent only starts on the first prompt, so NO snapshot is coming
// (`OpenSession` revives the agent but never snapshots; snapshots are sent only
// at connect-time, for sessions that already existed). Without this the transcript
// skeleton (`loading = !hydrated`) would spin forever on a freshly created
// session. Idempotent; a later real snapshot (e.g. after reload) just no-ops.
export function markSessionHydrated(id: string): void {
  const retry = sessionHydrationRetryTimers.get(id);
  if (retry !== undefined) clearTimeout(retry);
  sessionHydrationRetryTimers.delete(id);
  if (state.hydrated.has(id)) return;
  setState({ ...state, hydrated: new Set(state.hydrated).add(id) });
}

// --- Queue + draft commands -------------------------------------------------
//
// The daemon owns the authoritative state and broadcasts every queue/draft
// change. Content-bearing and destructive user intents are first staged in the
// IndexedDB outbox, then sent and replayed idempotently after reconnect. Bulk
// commands that already wait for an authoritative acknowledgement remain direct.

// The single entry point the composer calls to send a user prompt. The daemon
// dispatches it immediately when idle, else queues it. A prompt with at least
// one attachment is valid even with empty text; otherwise empty text is ignored.
export function submitPrompt(
  sessionId: string,
  text: string,
  attachments: Attachment[] = [],
): Promise<void> {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return Promise.resolve();
  // Predict the daemon's path so the optimistic row lands in the right place: it
  // DISPATCHES (→ a chat bubble) only when the session is dispatchable AND
  // nothing is queued; otherwise it QUEUES (→ a queue row). A wrong guess
  // self-heals — reconcile drops the row from whichever overlay the moment its
  // cmid lands server-side (queue broadcast OR chat echo).
  const sess = state.sessions.find((s) => s.id === sessionId);
  const dispatchable = sess !== undefined
    && ["running", "exited", "crashed", "interrupted"].includes(sess.status);
  // state.queues already includes optimistic queue rows (merged by commitQueue),
  // so this covers both server + local pending.
  const queueEmpty = (state.queues.get(sessionId)?.length ?? 0) === 0;
  if (shouldUseTranscriptDelivery(isConnected(), dispatchable, queueEmpty)) {
    // → dispatch: an optimistic CHAT bubble in the transcript.
    return optimisticMessage(sessionId, trimmed, attachments, "composer");
  } else {
    // → queue: an optimistic row in the queue sync state.
    return qAdd("queue", sessionId, trimmed, attachments, { origin: "composer" });
  }
}

/** Force-push send (the long-press affordance): when a turn is in flight, jump
 *  this prompt to the FRONT of the queue and interrupt the running turn so it
 *  runs next. On an idle session there's nothing to jump ahead of, so it's just
 *  a normal send (a chat bubble). Mirrors submitPrompt's optimistic placement. */
export function forcePrompt(sessionId: string, text: string, attachments: Attachment[] = []): Promise<void> {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return Promise.resolve();
  const sess = state.sessions.find((s) => s.id === sessionId);
  const dispatchable = sess !== undefined
    && ["running", "exited", "crashed", "interrupted"].includes(sess.status);
  const queueEmpty = (state.queues.get(sessionId)?.length ?? 0) === 0;
  if (shouldUseTranscriptDelivery(isConnected(), dispatchable, queueEmpty)) {
    // Idle → nothing to force ahead of; a normal optimistic chat send.
    return optimisticMessage(sessionId, trimmed, attachments, "composer");
  } else {
    // Busy → optimistic FRONT row + `submit { force: true }` (interrupt + run next).
    return qAdd("queue", sessionId, trimmed, attachments, { mode: "force", origin: "composer" });
  }
}

/** "Jump to front of queue" (no interrupt): when a turn is in flight with other
 *  messages already queued, send this prompt to the FRONT of the queue WITHOUT
 *  cancelling the running turn — it runs next (after the current turn) ahead of
 *  the rest of the queue. On an idle / empty-queue session there's nothing to
 *  jump ahead of, so it's just a normal send. Mirrors forcePrompt minus the
 *  interrupt. */
export function frontPrompt(sessionId: string, text: string, attachments: Attachment[] = []): Promise<void> {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return Promise.resolve();
  const sess = state.sessions.find((s) => s.id === sessionId);
  const dispatchable = sess !== undefined
    && ["running", "exited", "crashed", "interrupted"].includes(sess.status);
  const queueEmpty = (state.queues.get(sessionId)?.length ?? 0) === 0;
  if (shouldUseTranscriptDelivery(isConnected(), dispatchable, queueEmpty)) {
    return optimisticMessage(sessionId, trimmed, attachments, "composer");
  } else {
    return qAdd("queue", sessionId, trimmed, attachments, { mode: "front", origin: "composer" });
  }
}

// "Send now" on a queued row: the daemon sends it if it can take a turn this
// instant, otherwise moves it to the front to drain next.
function sessionDispatchable(sessionId: string): boolean {
  const sess = state.sessions.find((s) => s.id === sessionId);
  return sess !== undefined &&
    ["running", "exited", "crashed", "interrupted"].includes(sess.status);
}

function findQueued(sessionId: string, id: string): QueuedMessage | undefined {
  return (state.queues.get(sessionId) ?? []).find((message) => message.id === id);
}

function findDraft(sessionId: string, id: string): QueuedMessage | undefined {
  return (state.drafts.get(sessionId) ?? []).find((message) => message.id === id);
}

async function editPendingRow(
  target: "draft" | "queued",
  sessionId: string,
  id: string,
  text: string,
  attachments: Attachment[],
): Promise<void> {
  const source = target === "draft"
    ? findDraft(sessionId, id)
    : findQueued(sessionId, id);
  if (source === undefined) {
    throw new Error(`Cannot save ${target} edit because the source row is gone`);
  }
  const row: QueuedMessage = {
    ...source,
    text: text.trimEnd(),
    attachments,
  };
  const store = qClient(sessionId);
  await store.mutateDurably(
    target === "draft" ? "editDraft" : "editQueue",
    { id, row },
    newCmid(),
  );
  await waitForState(
    (snapshot) => {
      const rows = target === "draft"
        ? snapshot.drafts.get(sessionId)
        : snapshot.queues.get(sessionId);
      const current = rows?.find((message) => message.id === id);
      return current !== undefined && sameQueuedContent(current, row);
    },
    target === "draft" ? "Save draft edit" : "Save queued edit",
  );
}

export async function requestSendQueued(sessionId: string, id: string): Promise<void> {
  const row = findQueued(sessionId, id);
  if (row === undefined) return Promise.resolve();
  if (row.status !== undefined && row.cmid !== undefined) {
    if (destinationForPrompt(isConnected(), sessionDispatchable(sessionId), true) === "transcript") {
      await optimisticMessage(sessionId, row.text, row.attachments, "queue");
      await discardQueued(sessionId, row.cmid);
      return;
    }
    return waitForState(
      (snapshot) => (snapshot.queues.get(sessionId) ?? []).some((message) => message.id === id),
      "Send queued message",
    );
  }
  const opId = newCmid();
  const echoCmid = row.cmid ?? opId;
  const presented: QueuedMessage = {
    id: `opt-${opId}`,
    text: row.text,
    attachments: row.attachments,
    cmid: echoCmid,
    origin: "queue",
  };
  qStatus.set(opId, "pending");
  qStatus.set(echoCmid, "pending");
  return qClient(sessionId).mutateDurably(
    "sendQueued",
    { id, row: presented },
    opId,
  ).then(() =>
    waitForState(
      (snapshot) =>
        !(snapshot.queues.get(sessionId) ?? []).some((message) =>
          message.id === id
        ),
      "Send queued message",
    )
  );
}

// "Force push" a queued row: interrupt the running turn and run this prompt
// next. The daemon promotes it and cancels the in-flight turn (or just sends it
// if the session is already idle).
export async function forcePushQueued(sessionId: string, id: string): Promise<void> {
  const row = findQueued(sessionId, id);
  if (row === undefined) return Promise.resolve();
  if (row.status !== undefined && row.cmid !== undefined) {
    await qAdd("queue", sessionId, row.text, row.attachments, { mode: "force", origin: "queue" });
    await discardQueued(sessionId, row.cmid);
    return;
  }
  const opId = newCmid();
  const echoCmid = row.cmid ?? opId;
  const presented: QueuedMessage = {
    id: `opt-${opId}`,
    text: row.text,
    attachments: row.attachments,
    cmid: echoCmid,
    origin: "queue",
  };
  qStatus.set(opId, "pending");
  qStatus.set(echoCmid, "pending");
  return qClient(sessionId).mutateDurably(
    "forceQueued",
    { id, row: presented },
    opId,
  ).then(() =>
    waitForState(
      (snapshot) =>
        !(snapshot.queues.get(sessionId) ?? []).some((message) =>
          message.id === id
        ),
      "Force push queued message",
    )
  );
}

// Edit a queued prompt in place — text AND attachments. Clearing both removes it
// (handled server-side).
export function editQueued(
  sessionId: string,
  id: string,
  text: string,
  attachments: Attachment[],
): Promise<void> {
  return editPendingRow("queued", sessionId, id, text, attachments);
}

// Drop one queued prompt.
export function removeQueued(sessionId: string, id: string): Promise<void> {
  if (findQueued(sessionId, id) === undefined) return Promise.resolve();
  return qClient(sessionId).mutateDurably(
    "removeQueue",
    { id },
    newCmid(),
  ).then(() =>
    waitForState(
      (snapshot) =>
        !(snapshot.queues.get(sessionId) ?? []).some((message) =>
          message.id === id
        ),
      "Remove queued message",
    )
  );
}

// Drop a session's whole queue (the "Clear All" header action).
export function clearQueue(sessionId: string): Promise<void> {
  return sendWithAck(
    { type: "clear_queue", session_id: sessionId },
    (snapshot) => (snapshot.queues.get(sessionId)?.length ?? 0) === 0,
    "Clear queue",
  );
}

// "Clear conversation": reset the agent's context (fresh session/new) and
// destructively remove its prior transcript. The daemon emits a fresh
// `context_cleared` boundary after clearing memory + durable history. This is
// the Clear composer action — NOT a slash command (no agent exposes `clear`).
export function resetSession(sessionId: string): Promise<void> {
  beginConversationClear(sessionId);
  const previousBoundary = [...(state.timelines.get(sessionId) ?? [])]
    .reverse()
    .find((event) =>
      event.kind === "update" && event.update.sessionUpdate === "context_cleared"
    )?.seq ?? -1;
  return sendWithAck(
    { type: "reset_session", session_id: sessionId },
    (snapshot) =>
      (snapshot.timelines.get(sessionId) ?? []).some((event) =>
        event.seq > previousBoundary &&
        event.kind === "update" &&
        event.update.sessionUpdate === "context_cleared"
      ),
    "Clear conversation",
  );
}

// Manually PAUSE / RESUME the queue drain (the ⏸ toggle). Paused holds the
// auto-drain — queued messages don't advance even after the current turn ends —
// without interrupting the running turn. The daemon broadcasts the flag back, so
// every terminal reflects it within a round-trip (non-optimistic, like
// setPaused).
export function setPaused(sessionId: string, paused: boolean): void {
  send({ type: "set_paused", session_id: sessionId, paused });
}

/** Overlay "Retry": re-run the last prompt of an errored/crashed turn. */
export function retryTurn(sessionId: string): void {
  send({ type: "retry_turn", session_id: sessionId });
}

// Hold (or release, with `null`) the queue head for editing — pauses the daemon
// drain on EVERY terminal while one client edits, so the message isn't sent out
// from under the editor.
export function setQueueEditing(sessionId: string, id: string | null): void {
  send({ type: "set_queue_editing", session_id: sessionId, id });
}

// --- Draft operations -------------------------------------------------------

// Park the composer's content as a new draft (the "Draft" button). Shows the
// draft INSTANTLY (optimistic), then sends. WS open → `pending` (no shimmer yet,
// see SHIMMER_DELAY_MS); WS down → pending in the durable outbox until reconnect.
// Empty is ignored.
export function addDraft(sessionId: string, text: string, attachments: Attachment[]): Promise<void> {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return Promise.resolve();
  return qAdd("drafts", sessionId, trimmed, attachments);
}

// Edit a draft in place (same shape as editQueued). Clearing both fields drops it.
export function editDraft(
  sessionId: string,
  id: string,
  text: string,
  attachments: Attachment[],
): Promise<void> {
  return editPendingRow("draft", sessionId, id, text, attachments);
}

// Drop one draft.
export function removeDraft(sessionId: string, id: string): Promise<void> {
  if (findDraft(sessionId, id) === undefined) return Promise.resolve();
  return qClient(sessionId).mutateDurably(
    "removeDraft",
    { id },
    newCmid(),
  ).then(() =>
    waitForState(
      (snapshot) =>
        !(snapshot.drafts.get(sessionId) ?? []).some((message) =>
          message.id === id
        ),
      "Remove draft",
    )
  );
}

// Drop a session's whole draft list ("Clear All" on the drafts panel).
export function clearDrafts(sessionId: string): Promise<void> {
  return sendWithAck(
    { type: "clear_drafts", session_id: sessionId },
    (snapshot) => (snapshot.drafts.get(sessionId)?.length ?? 0) === 0,
    "Clear drafts",
  );
}

// Activate a draft: the daemon submits it (send-or-queue) and removes it from
// drafts.
export async function activateDraft(sessionId: string, id: string): Promise<void> {
  const row = findDraft(sessionId, id);
  if (row === undefined) return Promise.resolve();
  const dest = destinationForPrompt(
    isConnected(),
    sessionDispatchable(sessionId),
    (state.queues.get(sessionId)?.length ?? 0) === 0,
  );
  if (row.status !== undefined && row.cmid !== undefined) {
    if (dest === "transcript") {
      await optimisticMessage(sessionId, row.text, row.attachments, "draft");
    } else {
      await qAdd("queue", sessionId, row.text, row.attachments, { origin: "draft" });
    }
    await discardQueued(sessionId, row.cmid);
    return;
  }
  const opId = newCmid();
  const presented = dest === "queue"
    ? {
      id: `opt-${opId}`,
      text: row.text,
      attachments: row.attachments,
      cmid: opId,
      origin: "draft" as const,
    }
    : undefined;
  qStatus.set(opId, "pending");
  if (presented !== undefined) qStatus.set(presented.cmid, "pending");
  return qClient(sessionId).mutateDurably(
    "activateDraft",
    presented === undefined ? { id } : { id, row: presented },
    opId,
  ).then(() => {
    if (dest === "transcript") {
      const map = new Map(state.optimisticMessages);
      map.set(sessionId, [
        ...(map.get(sessionId) ?? []),
        {
          id: `opt-${opId}`,
          text: row.text,
          attachments: row.attachments,
          cmid: opId,
          status: statusAfterExplicitSend(isConnected()),
          origin: "draft",
        },
      ]);
      setState({ ...state, optimisticMessages: map });
      if (isConnected()) armMsgTimers(sessionId, opId);
    } else {
      revealPendingArrival({ kind: "queued", id: `opt-${opId}`, cmid: opId });
    }
    return waitForState(
      (snapshot) =>
        !(snapshot.drafts.get(sessionId) ?? []).some((message) =>
          message.id === id
        ),
      "Send draft",
    );
  });
}

// Send all drafts (front-to-back) — bulk "send everything" on the drafts panel.
export function activateAllDrafts(sessionId: string): Promise<void> {
  const drafts = [...(state.drafts.get(sessionId) ?? [])];
  return drafts.reduce(
    (previous, draft) => previous.then(() => activateDraft(sessionId, draft.id)),
    Promise.resolve(),
  );
}

// Give a draft a future fire time (or reschedule one). `id` targets an existing
// draft; omit it to CREATE a scheduled draft from the composer's content (a fresh
// cmid dedups a retry). The server auto-activates it at `fireAtMs` — fires even
// with every client offline. Both create and reschedule use the durable queue
// outbox so a PWA release between tapping Save and the server patch cannot lose
// intent.
export function scheduleDraft(
  sessionId: string,
  opts: { id?: string; text?: string; attachments?: Attachment[]; fireAtMs: number; delivery?: Delivery },
): Promise<void> {
  const { id, text, attachments, fireAtMs, delivery = "back" } = opts;
  const trimmed = (text ?? "").trimEnd();
  if (id === undefined) {
    return qAdd("scheduled", sessionId, trimmed, attachments ?? [], {
      schedule: { fire_at_ms: fireAtMs, delivery },
    });
  }
  const source = findDraft(sessionId, id);
  if (source === undefined) {
    return Promise.reject(new Error("Cannot schedule a draft that no longer exists"));
  }
  const row: QueuedMessage = {
    ...source,
    text: text === undefined ? source.text : trimmed,
    attachments: attachments ?? source.attachments,
    schedule: { fire_at_ms: fireAtMs, delivery },
  };
  return qClient(sessionId).mutateDurably(
    "rescheduleDraft",
    { id, row },
    newCmid(),
  ).then(() =>
    waitForState(
      (snapshot) =>
        snapshot.drafts.get(sessionId)?.some((draft) =>
          draft.id === id &&
          draft.schedule?.fire_at_ms === fireAtMs &&
          draft.schedule.delivery === delivery
        ) === true,
      "Schedule draft",
    )
  );
}

// Strip the schedule off a draft (it stays a plain parked draft).
export function unscheduleDraft(sessionId: string, id: string): Promise<void> {
  const source = findDraft(sessionId, id);
  if (source === undefined || source.schedule === undefined) {
    return Promise.resolve();
  }
  const row: QueuedMessage = {
    id: source.id,
    text: source.text,
    attachments: source.attachments,
    ...(source.cmid !== undefined ? { cmid: source.cmid } : {}),
    ...(source.origin !== undefined ? { origin: source.origin } : {}),
  };
  return qClient(sessionId).mutateDurably(
    "unscheduleDraft",
    { id, row },
    newCmid(),
  ).then(() =>
    waitForState(
      (snapshot) =>
        snapshot.drafts.get(sessionId)?.some((draft) =>
          draft.id === id && draft.schedule === undefined
        ) === true,
      "Cancel scheduled draft",
    )
  );
}

// Move a queued prompt back to drafts.
export async function queuedToDraft(sessionId: string, id: string): Promise<void> {
  const row = findQueued(sessionId, id);
  if (row === undefined) return Promise.resolve();
  if (row.status !== undefined && row.cmid !== undefined) {
    const text = row.text;
    const attachments = row.attachments;
    await qAdd("drafts", sessionId, text, attachments, { origin: "queue" });
    await discardQueued(sessionId, row.cmid);
    return;
  }
  const opId = newCmid();
  const presented: QueuedMessage = {
    id: row.id,
    text: row.text,
    attachments: row.attachments,
    ...(row.cmid !== undefined ? { cmid: row.cmid } : {}),
    origin: "queue",
    ...(row.schedule !== undefined ? { schedule: row.schedule } : {}),
  };
  qStatus.set(opId, "pending");
  return qClient(sessionId).mutateDurably(
    "returnQueuedToDraft",
    { id, row: presented },
    opId,
  ).then(() => {
    revealPendingArrival({
      kind: "draft",
      id: presented.id,
      ...(presented.cmid !== undefined ? { cmid: presented.cmid } : {}),
    });
  });
}

// Move a draft to another session's drafts (the "parked it in the wrong
// session" fix). Server-authoritative: the daemon relocates the whole message
// and broadcasts both sessions' panels, so every terminal stays in sync.
export function moveDraft(fromSession: string, id: string, toSession: string): void {
  send({ type: "move_draft", session_id: fromSession, id, to_session: toSession });
}

// --- Reorder (drag) ---------------------------------------------------------
// Optimistic via the "order" sync state: apply the new ordering locally (instant
// drag result) + send; the arbiter echoes a `sync_patch` every terminal folds.

export function reorderSessions(order: string[]): void {
  orderSync.mutate("reorder", { order });
}

export function reorderQueue(sessionId: string, order: string[]): void {
  send({ type: "reorder_queue", session_id: sessionId, order });
}

export function reorderDrafts(sessionId: string, order: string[]): void {
  send({ type: "reorder_drafts", session_id: sessionId, order });
}

function subscribe(listener: () => void): () => void {
  if (listeners.size === 0 && !socket && !productSessionAbandoned) connect();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

// Final flush of every IndexedDB sync cache on page hide/background. Normal use
// already debounce-saves, so this only captures the last sub-debounce change.
// Visibility arrives earlier than pagehide on iOS and gives transactions more
// time to commit; pagehide remains the final fallback.
if (typeof globalThis.addEventListener === "function") {
  const flushLocalState = (): void => {
    for (const entry of syncClients.values()) {
      void entry.flush().catch(() => undefined);
    }
    for (const client of qClients.values()) {
      void client.flush().catch(() => undefined);
    }
  };
  globalThis.addEventListener("pagehide", flushLocalState);
  globalThis.addEventListener("visibilitychange", () => {
    if (globalThis.document?.visibilityState === "hidden") flushLocalState();
  });
}

export function useStore(): State {
  return useSyncExternalStore(subscribe, () => presentedState);
}

/** Subscribe to one stable slice instead of re-rendering for every unrelated
 * store mutation. `getSnapshot` keeps the previous reference when the selected
 * value passes the supplied equality check, as required by
 * `useSyncExternalStore`; existing callers retain `Object.is` semantics. */
export function useStoreSelector<T>(
  selector: (snapshot: State) => T,
  equal: (previous: T, next: T) => boolean = Object.is,
): T {
  const selectorRef = useRef(selector);
  selectorRef.current = selector;
  const equalRef = useRef(equal);
  equalRef.current = equal;
  const cacheRef = useRef<{ value: T } | undefined>(undefined);
  const getSnapshot = useCallback((): T => {
    const next = selectorRef.current(presentedState);
    if (cacheRef.current && equalRef.current(cacheRef.current.value, next)) {
      return cacheRef.current.value;
    }
    cacheRef.current = { value: next };
    return next;
  }, []);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
