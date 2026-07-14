// Single WebSocket store shared by the whole app. cowboy is the source of
// truth; this store just accumulates what it pushes. Exposed via
// useSyncExternalStore so any component re-renders on change.
//
// All clients are equal subscribers: on connect the daemon sends the session
// list + a snapshot of each session's log, then a live tail. We dedup events
// by (session_id, seq) so a reconnect snapshot overlapping the live stream is
// harmless.

import { useCallback, useRef, useSyncExternalStore } from "react";
import { createConnectionStore } from "./_shell";
import {
  type ArgsOf,
  type ClientSnapshot,
  type Mutators,
  replicatedStore,
  type ReplicatedStore,
  snapshotPatch,
} from "./_sync/mod.ts";
import { idbListKeys, idbPersistence } from "./_sync-idb/mod.ts";
import { type Attachment, blocksToAttachments, buildContentBlocks } from "./attachments";
import { pruneDrafts } from "./draftStore";
import { linkTimeline } from "./derive";
import { notifyHaptic } from "./haptic";
import { fireAlert, vibrateAlertOn } from "./turnNotify";
import type {
  ConfigOption,
  ContentBlock,
  Delivery,
  DraftSchedule,
  Envelope,
  Inbound,
  JudgeResult,
  JudgeRun,
  Outbound,
  SessionMeta,
  SkillView,
  WireQueued,
} from "./protocol";
import { retainTimelineState } from "./timelineRetention";

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

/// Top-of-app connection / version banner — now the shared @shared-utils/ui
/// instance (see ./_shell connection-banner.tsx). cowboy adopts liveview's
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
  /** Present only on a DRAFT with a future fire time — the server auto-activates
   *  it then. Drives the draft-row clock chip. Absent on plain drafts. */
  schedule?: DraftSchedule;
}

export interface State {
  connected: boolean;
  sessions: SessionMeta[];
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
  // session_id → title from the "title" sync state (@shared-utils/sync). Source
  // of title truth on the client: overlaid onto SessionMeta.title by
  // `deriveSessions`, so an optimistic rename shows instantly and every terminal
  // converges on the arbiter's `sync_patch`. Ids absent here fall back to the
  // broadcast SessionMeta.title.
  titleOverrides: Record<string, string>;
  // Global key-value settings (auto-resume default flag + continuation template),
  // from the `settings` broadcast. Drives the Settings UI + the per-session badge
  // (effective auto-resume = session override ?? this default).
  settings: Record<string, unknown>;
  // The static skill registry (prompt + extract) from the `skills` broadcast.
  skills: SkillView[];
  // Latest confirm-detect judge result per session (drives the overlay's raw-data
  // expand). Keyed by session id.
  judgeResults: Record<string, JudgeResult>;
  // The confirm-detect judge-run HISTORY per session (newest first, capped),
  // server-authoritative + persisted. Backs the inspector widget (long-press the
  // turn-status pill). Keyed by session id; populated by the `judge_history`
  // broadcast (connect seed + every add/delete/clear).
  judgeRuns: Record<string, JudgeRun[]>;
}

let errorSeq = 0;
let state: State = {
  connected: false,
  sessions: [],
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
  settings: {},
  skills: [],
  judgeResults: {},
  judgeRuns: {},
};
const listeners = new Set<() => void>();
let socket: WebSocket | undefined;
// The session the user currently has open. Remembered so every (re)connect can
// re-assert it to the daemon (revive-on-open), recovering the agent after a
// daemon restart we reconnected across. See openSession + connect's onopen.
let openedSessionId: string | undefined;

// --- Reconnect bookkeeping --------------------------------------------------
// The banner + version-probe + outage/reconnect-flash policy now live in `conn`
// (the shared @shared-utils/ui connection store). The store just reports socket
// open/close into it and reads back the backoff delay. The only reconnect state
// kept here is the single pending-attempt timer.
let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

// --- Liveness watchdog (half-open detection) --------------------------------
// A WebSocket can go HALF-OPEN — TCP still "connected" but no data flows and
// `onclose` NEVER fires — which is common on mobile/5G (NAT drops, radio
// handoffs, app suspend). The socket then silently stops delivering updates and
// the UI freezes at the last-seen state (e.g. a status stuck mid-turn → a
// spinner that never resolves). The daemon sends an app-level heartbeat
// (Outbound::Ping) on a fixed interval, so on a HEALTHY socket some message
// always arrives within it; prolonged silence means the socket is dead. We
// track the last-message time and, if it goes stale, force-close → `onclose` →
// reconnect → fresh snapshot (self-healing the frozen state).
const STALE_MS = 60_000; // ~2.4 missed 25s heartbeats → dead (conservative)
const FOREGROUND_STALE_MS = 30_000; // on app-foreground, suspend likely killed it
const LIVENESS_CHECK_MS = 15_000;
let lastMessageAt = 0;
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
      ws.close(); // → onclose → scheduleReconnect → fresh snapshot
    }
  }, LIVENESS_CHECK_MS);
}

// Mobile suspends a backgrounded tab (freezing timers AND often killing the
// socket without an `onclose`); on return the socket may be a zombie. Timers
// were frozen, so the watchdog above hasn't run — probe immediately on
// foreground and reconnect if we haven't heard from the server recently.
if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    if (
      document.visibilityState === "visible" && socket &&
      socket.readyState === WebSocket.OPEN &&
      Date.now() - lastMessageAt > FOREGROUND_STALE_MS
    ) {
      socket.close();
    }
  });
}

function emit(): void {
  scheduleNotify();
}

// WebSocket chunks can arrive much faster than the display can paint. State is
// still reduced synchronously (no event is lost), but subscriber rendering is
// capped at ~30fps. A model transcript gains no useful fidelity at 60–100
// React/layout commits per second; sustained multi-session streams otherwise
// make Desktop progressively sticky while competing with editor input. In a
// background tab rAF may be suspended, so a coarse timer keeps state observers
// progressing without burning CPU.
const FOREGROUND_NOTIFY_INTERVAL_MS = 33;
let notifyScheduled = false;
let lastNotifyAt = 0;
function flushNotify(): void {
  notifyScheduled = false;
  lastNotifyAt = performance.now();
  for (const listener of listeners) listener();
}
function scheduleNotify(): void {
  if (notifyScheduled) return;
  notifyScheduled = true;
  if (typeof document !== "undefined" && document.visibilityState === "visible") {
    const remaining = Math.max(
      0,
      FOREGROUND_NOTIFY_INTERVAL_MS - (performance.now() - lastNotifyAt),
    );
    if (remaining === 0) {
      requestAnimationFrame(flushNotify);
    } else {
      setTimeout(() => requestAnimationFrame(flushNotify), remaining);
    }
  } else {
    setTimeout(flushNotify, 50);
  }
}

function setState(next: State): void {
  state = next;
  emit();
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

  // These high-frequency frames are runtime telemetry, not transcript history.
  // The daemon persists their sequence watermark without storing a row; mirror
  // that canonical representation in the live client so long turns don't grow
  // one inert object per token update.
  if (
    env.kind === "update" &&
    (env.update.sessionUpdate === "usage_update" ||
      env.update.sessionUpdate === "session_info_update")
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

// Bulk merge a run of events (a snapshot tail or an older history page) into one
// session's timeline: dedup by seq, keep seq-ordered, in a single pass (vs
// applyEnvelope per event). Overlap between an unaligned tail and an aligned
// page is harmless — the dedup drops it.
function mergeEvents(
  timelines: Map<string, Envelope[]>,
  sessionId: string,
  events: Envelope[],
): Map<string, Envelope[]> {
  const next = new Map(timelines);
  const existing = next.get(sessionId) ?? [];
  const fresh = events.filter((event) => !containsSeq(existing, event.seq));
  if (fresh.length === 0 && existing.length > 0) return next;
  const merged = linkTimeline(
    [...existing, ...fresh].sort((a, b) => a.seq - b.seq),
    existing,
  );
  next.set(sessionId, merged);
  return next;
}

// History page size — MUST match the server's HISTORY_PAGE (src/core.rs). Pages
// are seq-aligned so each has a stable, cacheable URL.
// Fetch the next OLDER page of a session's history and prepend it. Pages come
// from the immutable HTTP route (GET /api/history/:id/:page) so a re-fetch
// (scroll back, reload, post-recycle) is a cache hit — zero network. One fetch
// at a time per session; no-op once the window already reaches the first event.
//
// VERSION-SCOPED url (`?v=<build>`): the cache is `immutable`, so without this a
// redeploy could keep serving pages cached under the OLD build (if a future
// version ever changes the event shape, that's stale/incompatible). Keying the
// url on the build id means a new version's pages are fresh fetches, while
// reloads of the SAME build stay cache hits. (`conn.version()` is the id the tab
// loaded against; until the first /version probe lands it's a harmless "0".)
export async function loadOlder(sessionId: string): Promise<void> {
  const pg = state.pagination.get(sessionId);
  if (!pg || pg.reachedStart || pg.loadingOlder || pg.beforeSeq === null) return;
  const beforeSeq = pg.beforeSeq;
  setPagination(sessionId, { ...pg, loadingOlder: true });
  try {
    const res = await fetch(
      `/api/history/${encodeURIComponent(sessionId)}?before_seq=${String(beforeSeq)}&v=${encodeURIComponent(conn.version() ?? "0")}`,
    );
    if (!res.ok) {
      setPagination(sessionId, { ...pg, loadingOlder: false });
      return;
    }
    const data = (await res.json()) as {
      events: Envelope[];
      next_before_seq: number | null;
      reached_start: boolean;
    };
    setState({ ...state, timelines: mergeEvents(state.timelines, sessionId, data.events) });
    // Always step to the next OLDER page (don't recompute from the oldest seq —
    // a gap at a boundary would re-request the same page forever). Page 0 was
    // the last → reached start.
    setPagination(sessionId, {
      reachedStart: data.reached_start,
      loadingOlder: false,
      beforeSeq: data.next_before_seq,
    });
  } catch {
    setPagination(sessionId, { ...pg, loadingOlder: false });
  }
}

const INACTIVE_HISTORY_TAIL = 800;
// The open transcript used to grow forever between page reloads. Keep a smaller
// recent window once a following reader crosses this batched high-water mark.
// The complete log remains in Postgres and `loadOlder` pages it back on demand.
const ACTIVE_HISTORY_TAIL = 120;
const ACTIVE_HISTORY_HIGH_WATER = 200;

function releaseHistoryTail(sessionId: string, retain: number): boolean {
  const timeline = state.timelines.get(sessionId);
  if (!timeline || timeline.length <= retain) return false;
  // Do not link the smaller tail back to the full timeline: this operation must
  // let deep history and its rendered Markdown become collectible.
  const retained = retainTimelineState(timeline, retain);
  const timelines = new Map(state.timelines);
  timelines.set(sessionId, retained.events);
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
  return releaseHistoryTail(sessionId, ACTIVE_HISTORY_TAIL);
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
      // The list is authoritative: drop composer drafts for sessions that no
      // longer exist (deleted here or on another terminal). Tolerant + off the
      // input path.
      pruneDrafts(new Set(msg.sessions.map((s) => s.id)));
      break;
    }
    case "snapshot": {
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
      setState({ ...state, timelines, hydrated, pagination });
      break;
    }
    case "event": {
      const env = msg.envelope;
      // Attention alert — a permission request needs a DECISION. A plain `turn_end`
      // is NOT alerted here: a finished turn might be done, still-working, or a
      // force-push landing — only the confirm-detect verdict (case "judge_result")
      // knows which, so the done/decision chimes fire there. These are LIVE events;
      // snapshot/history replays go through `case "snapshot"`, so past turns never
      // re-ding. `fireAlert` no-ops when the setting is off / the tab is visible.
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
      setState({
        ...state,
        timelines: applyEnvelope(state.timelines, env),
        ...(cmid !== undefined && {
          optimisticMessages: reconcileOptimistic(state.optimisticMessages, env.session_id, new Set([cmid])),
        }),
      });
      break;
    }
    case "config_options": {
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
        syncClients.get(msg.state)?.applyPatch(msg.version, msg.value, msg.confirmed, resync);
      }
      break;
    }
    case "settings": {
      setState({ ...state, settings: msg.settings });
      break;
    }
    case "skills": {
      setState({ ...state, skills: msg.skills });
      break;
    }
    case "judge_result": {
      setState({ ...state, judgeResults: { ...state.judgeResults, [msg.session_id]: msg } });
      // The semantic attention alert: the verdict is what makes a turn-end worth a
      // sound. `done` → the completion chime; `awaiting_user` (the agent asked
      // something) → the decision chime. A plain continue / a force-push lands here
      // with both false → no sound. (The provisional hold doesn't send a
      // judge_result — only a real L1/L2 verdict does.)
      // Haptic on the SAME semantic turn-end the chime fires on — gated on the
      // independent vibration setting (separate from sound), but NOT on tab-hidden:
      // a native haptic only registers while the app is foreground, which is exactly
      // when the "it finished / it needs you" buzz is wanted.
      if (msg.done) {
        fireAlert("done");
        if (vibrateAlertOn()) notifyHaptic("success");
      } else if (msg.awaiting_user) {
        fireAlert("decision");
        if (vibrateAlertOn()) notifyHaptic("warning");
      }
      break;
    }
    case "judge_history": {
      setState({ ...state, judgeRuns: { ...state.judgeRuns, [msg.session_id]: msg.runs } });
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
function scheduleReconnect(delay: number): void {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = undefined;
    connect();
  }, delay);
}

// Hydrate the locally-cached sync states (title/order) from IndexedDB ONCE, then
// open the socket. Instant-load: the last-known titles/order paint before the
// first server byte. Reconnects skip this — the in-memory state is already fresh
// and newer than the debounce-saved cache, so re-hydrating would stomp it.
let didHydrate = false;
function connect(): void {
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
      await Promise.all([...syncClients.values()].map((e) => e.hydrate()));
      await hydrateCachedQueues();
    } catch (err) {
      console.warn("sync hydrate failed", err);
    } finally {
      clearTimeout(grace);
      openOnce();
    }
  })();
}

function openSocket(): void {
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
  const ws = new WebSocket(`${proto}//${globalThis.location.host}/ws`);
  socket = ws;
  // A socket wedged in CONNECTING (a half-open proxy / network that completes the
  // TCP handshake but never the WS upgrade) fires NEITHER onopen NOR onclose, so
  // without this it strands the UI on "Connecting…" forever with no reconnect.
  // Force it closed after a grace → onclose → scheduleReconnect. Cleared the moment
  // it opens or closes on its own.
  const connectGuard = setTimeout(() => {
    if (socket === ws && ws.readyState === WebSocket.CONNECTING) {
      ws.close();
    }
  }, 8000);
  ws.onopen = (): void => {
    clearTimeout(connectGuard);
    if (socket !== ws) {
      ws.close();
      return;
    }
    if (reconnectTimer !== undefined) {
      clearTimeout(reconnectTimer);
      reconnectTimer = undefined;
    }
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
  ws.onclose = (): void => {
    clearTimeout(connectGuard);
    // A superseded socket may close after its replacement has opened. It no
    // longer owns global connection state and must not raise the red banner or
    // schedule another reconnect.
    if (socket !== ws) return;
    socket = undefined;
    stopLiveness();
    setState({ ...state, connected: false });
    // Raises the red banner past the failure threshold and hands back the
    // exponential-backoff delay to wait before retrying (banner lives in `conn`).
    scheduleReconnect(conn.connectionLost());
  };
  ws.onerror = (): void => {
    if (socket === ws) ws.close();
  };
}

/** Returns whether the command actually went out (socket OPEN). The optimistic
 *  draft path uses this: a `false` means the send never left this device, so the
 *  row goes straight to `failed` (retry from here) instead of `sending`. */
export function send(cmd: Inbound): boolean {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(cmd));
    return true;
  }
  return false;
}

/** Whether a `send` right now would actually leave the device (socket OPEN).
 *  Lets a caller that routes its send through a sync store's `send` callback
 *  still learn the outcome for optimistic status (same check `send` makes). */
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
  const c = globalThis.crypto;
  if (c?.randomUUID) return c.randomUUID();
  return `c-${String(Date.now())}-${Math.random().toString(36).slice(2)}`;
}

// --- Optimistic sync (state-sync engine: @shared-utils/sync) -----------------
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
): { view: () => T; mutate: <K extends keyof M & string>(name: K, args: ArgsOf<T, M, K>) => void } {
  const store = replicatedStore<T, M>({
    clientId: `${syncBase}:${syncState}`,
    mutators,
    initial,
    send: (m): void => {
      send({ type: "sync", state: syncState, id: m.id, name: m.name, args: m.args });
    },
    onChange: commitSessions,
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

// --- Auto-resume interrupted turns (tasks/archive/2026/07/session-auto-resume) ---

export const AUTO_RESUME_DEFAULT_KEY = "session.autoResume.default";
export const AUTO_RESUME_TEMPLATE_KEY = "session.autoResume.template";
/** The built-in continuation template (mirrors DEFAULT_CONTINUATION_TEMPLATE in
 *  src/core.rs) — shown in the editor when the operator hasn't customized one. */
export const DEFAULT_CONTINUATION_TEMPLATE =
  "[系统自动续接,非用户重新提问] 你上一轮回复在完成前被 cowboy 重启打断,系统现自动恢复该轮。请**从中断处接着完成**,不要从头重做整个任务;尤其在重新执行任何有副作用的操作(写/改文件、部署、git 提交、发网络请求等)之前,先确认它是否已经做过,避免重复执行导致循环或副作用叠加。以下是你被打断前已产出的内容:\n\n{{partial}}";

/** The global auto-resume default (off unless explicitly enabled). */
export function autoResumeDefault(s: State): boolean {
  return s.settings[AUTO_RESUME_DEFAULT_KEY] === true;
}

/** Effective auto-resume for a session: its override, else the global default. */
export function effectiveAutoResume(meta: SessionMeta, s: State): boolean {
  return meta.auto_resume ?? autoResumeDefault(s);
}

/** Set one global setting. Optimistic (plain field) + send; the daemon persists
 *  + re-broadcasts, which reconciles. */
export function setSetting(key: string, value: unknown): void {
  setState({ ...state, settings: { ...state.settings, [key]: value } });
  send({ type: "set_setting", key, value });
}

/** The registered skills (prompt + extract), for the Info sheet viewer. */
export function useSkills(): SkillView[] {
  return useStoreSelector((snapshot) => snapshot.skills);
}

/** Whether the live WS is currently open. False the moment it drops (onclose),
 *  true on (re)open — drives the turn-status pill's "Reconnecting…" state. */
export function useConnected(): boolean {
  return useStoreSelector((snapshot) => snapshot.connected);
}

/** Set a session's auto-resume override (`null` = inherit the global default).
 *  Non-optimistic: the daemon re-broadcasts the SessionMeta within a round-trip
 *  (the override rides the raw session list, which `deriveSessions` rebuilds). */
export function setSessionAutoResume(sessionId: string, value: boolean | null): void {
  send({ type: "set_session_auto_resume", session_id: sessionId, value });
}

// --- Queue + drafts optimistic sync (per session) ----------------------------
// Each session's queue+drafts is a sync state "queue:<sid>". The daemon is the
// arbiter (its typed queue logic unchanged); the client uses the shared engine
// for instant optimistic ADDs (addDraft / submit-when-busy) that rebase onto the
// latest server list and reconcile by cmid (no ghost). Non-add ops (remove /
// edit / clear / reorder / move) stay plain commands — the daemon applies them
// and the next patch reflects them (no client mutator → no pending → no risk).
// pending/sending/failed STATUS is local presentation, kept in `qStatus` (keyed
// by cmid) and merged at commit — never in the synced value.

interface QValue {
  readonly queue: readonly QueuedMessage[];
  readonly drafts: readonly QueuedMessage[];
}
const qMut = {
  addDraft: (v: QValue, a: { row: QueuedMessage }): QValue => ({ ...v, drafts: [...v.drafts, a.row] }),
  addQueue: (v: QValue, a: { row: QueuedMessage }): QValue => ({ ...v, queue: [...v.queue, a.row] }),
  // Force-push (long-press send): land the optimistic row at the FRONT of the
  // queue, matching where the daemon puts it; its wire frame is `submit` with
  // `force: true`, so the daemon also interrupts the running turn.
  forceQueue: (v: QValue, a: { row: QueuedMessage }): QValue => ({ ...v, queue: [a.row, ...v.queue] }),
  // "Jump to front" (no interrupt): same FRONT placement as forceQueue, but its
  // wire frame is `submit` with `front: true` — the daemon puts it ahead of the
  // queue WITHOUT cancelling the running turn (runs next after the current turn).
  frontQueue: (v: QValue, a: { row: QueuedMessage }): QValue => ({ ...v, queue: [a.row, ...v.queue] }),
} satisfies Mutators<QValue>;
const qClients = new Map<string, ReplicatedStore<QValue, typeof qMut>>();
const qStatus = new Map<string, "pending" | "sending" | "failed">();

function qClient(sessionId: string): ReplicatedStore<QValue, typeof qMut> {
  let c = qClients.get(sessionId);
  if (c === undefined) {
    c = replicatedStore<QValue, typeof qMut>({
      clientId: `${syncBase}:queue:${sessionId}`,
      mutators: qMut,
      initial: { queue: [], drafts: [] },
      // An optimistic add's wire frame is the daemon's own add_draft/submit
      // command (cmid = the mutation id ⇒ the server echo confirms exactly this
      // row, no ghost). Used both for the initial send (via mutate) and the
      // reconnect resend (via resend).
      send: (m): void => {
        const row = (m.args as { row: QueuedMessage }).row;
        send(
          m.name === "addDraft"
            ? { type: "add_draft", session_id: sessionId, text: row.text, content: contentOf(row.text, row.attachments), cmid: m.id }
            : {
              type: "submit",
              session_id: sessionId,
              text: row.text,
              content: contentOf(row.text, row.attachments),
              cmid: m.id,
              // `forceQueue` carries the force flag so a reconnect resend still
              // force-pushes; `frontQueue` carries `front` (jump ahead, no
              // interrupt); `addQueue` omits both (normal queue append).
              ...(m.name === "forceQueue" && { force: true }),
              ...(m.name === "frontQueue" && { front: true }),
            },
        );
      },
      onChange: (): void => {
        commitQueue(sessionId);
      },
      // Durable outbox: cache {base, pending} per session. On reload we enumerate
      // these keys and hydrate each qClient BEFORE the socket opens (see
      // connect()), so a staged/queued message painted instantly survives the
      // reload and re-sends; the per-session `queue_resync` (force) that follows
      // is the authority that corrects any stale cached base.
      local: idbPersistence<ClientSnapshot<QValue>>(`cowboy:sync:queue:${sessionId}`),
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
  await Promise.all(
    keys.filter((k) => k.startsWith(prefix)).map((k) => qClient(k.slice(prefix.length)).hydrate()),
  );
}

/** Render queue + drafts for one session: the sync client's view (server base +
 *  rebased optimistic adds), with the local `status` merged onto each still-
 *  pending optimistic row. */
function commitQueue(sessionId: string): void {
  const c = qClients.get(sessionId);
  const view: QValue = c ? c.get() : { queue: [], drafts: [] };
  const pend = new Set((c?.pending() ?? []).map((m) => m.id));
  const withStatus = (rows: readonly QueuedMessage[]): QueuedMessage[] =>
    rows.map((r) => (r.cmid !== undefined && pend.has(r.cmid) ? { ...r, status: qStatus.get(r.cmid) ?? "pending" } : r));
  const queues = new Map(state.queues);
  const drafts = new Map(state.drafts);
  const q = withStatus(view.queue);
  const d = withStatus(view.drafts);
  if (q.length > 0) queues.set(sessionId, q);
  else queues.delete(sessionId);
  if (d.length > 0) drafts.set(sessionId, d);
  else drafts.delete(sessionId);
  setState({ ...state, queues, drafts });
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
function qAdd(
  target: "drafts" | "queue",
  sessionId: string,
  text: string,
  attachments: Attachment[],
  // queue placement: "back" (normal append), "front" (jump ahead, no interrupt),
  // "force" (jump ahead + interrupt the running turn). Ignored for drafts.
  mode: "back" | "front" | "force" = "back",
): void {
  const cmid = newCmid();
  const row: QueuedMessage = { id: `opt-${cmid}`, text, attachments, cmid };
  const store = qClient(sessionId);
  // Set status BEFORE mutating: the mutate auto-sends the add_draft/submit frame
  // (the store's `send`) AND fires `onChange` → commitQueue, which reads this
  // status. `isConnected()` predicts the send's success (same socket check).
  const sent = isConnected();
  qStatus.set(cmid, sent ? "pending" : "failed");
  const mutator = target === "drafts"
    ? "addDraft"
    : mode === "force"
    ? "forceQueue"
    : mode === "front"
    ? "frontQueue"
    : "addQueue";
  store.mutate(mutator, { row }, cmid);
  if (sent) armQTimers(sessionId, cmid);
}

/** Retry a failed optimistic queue/draft row from THIS device: re-anchor it to
 *  the END (bump) then resend (same cmid ⇒ idempotent). */
export function retryQueued(sessionId: string, cmid: string): void {
  const c = qClients.get(sessionId);
  if (c === undefined) return;
  const view = c.get();
  const inDrafts = view.drafts.some((r) => r.cmid === cmid);
  const row = (inDrafts ? view.drafts : view.queue).find((r) => r.cmid === cmid);
  if (row === undefined) return;
  c.bump(cmid);
  const cmd: Inbound = inDrafts
    ? { type: "add_draft", session_id: sessionId, text: row.text, content: contentOf(row.text, row.attachments), cmid }
    : { type: "submit", session_id: sessionId, text: row.text, content: contentOf(row.text, row.attachments), cmid };
  const sent = send(cmd);
  qStatus.set(cmid, sent ? "pending" : "failed");
  commitQueue(sessionId);
  if (sent) armQTimers(sessionId, cmid);
}

/** Discard a (failed) optimistic queue/draft row locally — it never reached the
 *  daemon, so nothing server-side to remove. */
export function discardQueued(sessionId: string, cmid: string): void {
  clearOptTimers(cmid);
  qStatus.delete(cmid);
  qClients.get(sessionId)?.confirm([cmid]);
  commitQueue(sessionId);
}

/** Fold a queue `sync_patch` for one session: apply (force on resync), clear the
 *  status/timers of any cmid the server confirmed, and cross-signal the chat
 *  overlay (a queue cmid landing also drops a wrong-guessed optimistic bubble). */
function applyQueuePatch(sessionId: string, version: number, value: unknown, confirmed: string[], resync: boolean): void {
  const wire = value as { queue?: WireQueued[]; drafts?: WireQueued[] };
  const next: QValue = { queue: fromWire(wire.queue ?? []), drafts: fromWire(wire.drafts ?? []) };
  qClient(sessionId).applyPatch(snapshotPatch(version, next, confirmed), { force: resync });
  for (const cmid of confirmed) {
    if (qStatus.has(cmid)) {
      clearOptTimers(cmid);
      qStatus.delete(cmid);
    }
  }
  if (confirmed.length > 0) {
    const set = new Set<string | undefined>(confirmed);
    setState({ ...state, optimisticMessages: reconcileOptimistic(state.optimisticMessages, sessionId, set) });
  }
  commitQueue(sessionId);
}

// The optimistic CHAT-bubble overlay (submit-when-idle → transcript) is the one
// append-optimistic path that isn't a small-value sync state — kept here, keyed
// by cmid, reconciled by the user-echo Envelope (or a wrong-guess queue patch).

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
  const sent = send({
    type: "submit",
    session_id: sessionId,
    text: row.text,
    content: contentOf(row.text, row.attachments),
    cmid,
  });
  patchMessage(sessionId, cmid, (m) => ({ ...m, status: sent ? "pending" : "failed" }));
  if (sent) armMsgTimers(sessionId, cmid);
}

/** Discard a (usually failed) optimistic chat bubble locally — it never reached
 *  the daemon, so there's nothing server-side to remove. */
export function discardMessage(sessionId: string, cmid: string): void {
  clearOptTimers(cmid);
  patchMessage(sessionId, cmid, "drop");
}

/** Optimistic chat send (submit-when-idle): show a bubble in the transcript,
 *  fire the submit, arm timers. WS open → `pending`; WS down → `failed`. */
function optimisticMessage(sessionId: string, text: string, attachments: Attachment[]): void {
  const cmid = newCmid();
  const sent = send({ type: "submit", session_id: sessionId, text, content: contentOf(text, attachments), cmid });
  const map = new Map(state.optimisticMessages);
  const row: QueuedMessage = { id: `opt-${cmid}`, text, attachments, cmid, status: sent ? "pending" : "failed" };
  map.set(sessionId, [...(map.get(sessionId) ?? []), row]);
  setState({ ...state, optimisticMessages: map });
  if (sent) armMsgTimers(sessionId, cmid);
}

// Tell the daemon the user opened/selected `id` so it revives that session's
// agent before the user types — design §7 "revive on open". Remembered so the
// id is re-asserted on every reconnect (see connect's onopen). Cheap + a
// server-side no-op when the agent is already alive, so it's fine to call on
// every navigation.
export function openSession(id: string): void {
  openedSessionId = id;
  send({ type: "open_session", session_id: id });
}

// Mark a session hydrated WITHOUT waiting for a server snapshot — called the
// moment the client creates a new session. A brand-new session has no history
// and the agent only starts on the first prompt, so NO snapshot is coming
// (`OpenSession` revives the agent but never snapshots; snapshots are sent only
// at connect-time, for sessions that already existed). Without this the transcript
// skeleton (`loading = !hydrated`) would spin forever on a freshly created
// session. Idempotent; a later real snapshot (e.g. after reload) just no-ops.
export function markSessionHydrated(id: string): void {
  if (state.hydrated.has(id)) return;
  setState({ ...state, hydrated: new Set(state.hydrated).add(id) });
}

// --- Queue + draft commands -------------------------------------------------
//
// Every op below is a thin command sent to the daemon, which owns the state and
// echoes the new queue/drafts back via a `queues` broadcast (so all terminals
// update). No optimistic local mutation, no drain, no serialization here — that
// all moved server-side. The `status` arg the UI used to thread for the
// send-vs-queue decision is gone: the daemon decides authoritatively.

// The single entry point the composer calls to send a user prompt. The daemon
// dispatches it immediately when idle, else queues it. A prompt with at least
// one attachment is valid even with empty text; otherwise empty text is ignored.
export function submitPrompt(sessionId: string, text: string, attachments: Attachment[] = []): void {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return;
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
  if (dispatchable && queueEmpty) {
    // → dispatch: an optimistic CHAT bubble in the transcript.
    optimisticMessage(sessionId, trimmed, attachments);
  } else {
    // → queue: an optimistic row in the queue sync state.
    qAdd("queue", sessionId, trimmed, attachments);
  }
}

/** Force-push send (the long-press affordance): when a turn is in flight, jump
 *  this prompt to the FRONT of the queue and interrupt the running turn so it
 *  runs next. On an idle session there's nothing to jump ahead of, so it's just
 *  a normal send (a chat bubble). Mirrors submitPrompt's optimistic placement. */
export function forcePrompt(sessionId: string, text: string, attachments: Attachment[] = []): void {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return;
  const sess = state.sessions.find((s) => s.id === sessionId);
  const dispatchable = sess !== undefined
    && ["running", "exited", "crashed", "interrupted"].includes(sess.status);
  const queueEmpty = (state.queues.get(sessionId)?.length ?? 0) === 0;
  if (dispatchable && queueEmpty) {
    // Idle → nothing to force ahead of; a normal optimistic chat send.
    optimisticMessage(sessionId, trimmed, attachments);
  } else {
    // Busy → optimistic FRONT row + `submit { force: true }` (interrupt + run next).
    qAdd("queue", sessionId, trimmed, attachments, "force");
  }
}

/** "Jump to front of queue" (no interrupt): when a turn is in flight with other
 *  messages already queued, send this prompt to the FRONT of the queue WITHOUT
 *  cancelling the running turn — it runs next (after the current turn) ahead of
 *  the rest of the queue. On an idle / empty-queue session there's nothing to
 *  jump ahead of, so it's just a normal send. Mirrors forcePrompt minus the
 *  interrupt. */
export function frontPrompt(sessionId: string, text: string, attachments: Attachment[] = []): void {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return;
  const sess = state.sessions.find((s) => s.id === sessionId);
  const dispatchable = sess !== undefined
    && ["running", "exited", "crashed", "interrupted"].includes(sess.status);
  const queueEmpty = (state.queues.get(sessionId)?.length ?? 0) === 0;
  if (dispatchable && queueEmpty) {
    optimisticMessage(sessionId, trimmed, attachments);
  } else {
    qAdd("queue", sessionId, trimmed, attachments, "front");
  }
}

// "Send now" on a queued row: the daemon sends it if it can take a turn this
// instant, otherwise moves it to the front to drain next.
export function requestSendQueued(sessionId: string, id: string): void {
  send({ type: "request_send_queued", session_id: sessionId, id });
}

// "Force push" a queued row: interrupt the running turn and run this prompt
// next. The daemon promotes it and cancels the in-flight turn (or just sends it
// if the session is already idle).
export function forcePushQueued(sessionId: string, id: string): void {
  send({ type: "force_push_queued", session_id: sessionId, id });
}

// Edit a queued prompt in place — text AND attachments. Clearing both removes it
// (handled server-side).
export function editQueued(
  sessionId: string,
  id: string,
  text: string,
  attachments: Attachment[],
): void {
  const trimmed = text.trimEnd();
  send({ type: "edit_queued", session_id: sessionId, id, text: trimmed, content: contentOf(trimmed, attachments) });
}

// Drop one queued prompt.
export function removeQueued(sessionId: string, id: string): void {
  send({ type: "remove_queued", session_id: sessionId, id });
}

// Drop a session's whole queue (the "Clear All" header action).
export function clearQueue(sessionId: string): void {
  send({ type: "clear_queue", session_id: sessionId });
}

// "Clear conversation": reset the agent's context (fresh session/new) while
// keeping the transcript. The daemon respawns the agent + emits a
// `context_cleared` marker the transcript renders as a divider. This is the
// Clear composer action — NOT a slash command (no agent exposes `clear`).
export function resetSession(sessionId: string): void {
  send({ type: "reset_session", session_id: sessionId });
}

// Lift the confirm-detect "awaiting user" hold (the awaiting widget's dismiss /
// Send). `false` = "the agent wasn't really asking" → the queue drains. Non-
// optimistic: the daemon `broadcast_sessions()` reflects the cleared flag within
// a round-trip (mirrors `setSessionAutoResume`).
export function dismissAwaiting(sessionId: string): void {
  send({ type: "set_awaiting", session_id: sessionId, awaiting: false });
}

// Manually PAUSE / RESUME the queue drain (the ⏸ toggle). Paused holds the
// auto-drain — queued messages don't advance even after the current turn ends —
// without interrupting the running turn. The daemon broadcasts the flag back, so
// every terminal reflects it within a round-trip (non-optimistic, like
// dismissAwaiting).
export function setPaused(sessionId: string, paused: boolean): void {
  send({ type: "set_paused", session_id: sessionId, paused });
}

/** The latest confirm-detect judge result for a session (overlay raw-data expand). */
export function useJudgeResult(sessionId: string): JudgeResult | undefined {
  return useStoreSelector((snapshot) => snapshot.judgeResults[sessionId]);
}

const EMPTY_RUNS: JudgeRun[] = [];

/** A session's confirm-detect judge-run history (newest first), for the inspector
 *  widget. Server-authoritative — stable empty array when none yet. */
export function useJudgeRuns(sessionId: string): JudgeRun[] {
  return useStoreSelector((snapshot) => snapshot.judgeRuns[sessionId] ?? EMPTY_RUNS);
}

/** Delete one judge run from a session's inspector history (server-authoritative;
 *  the `judge_history` re-broadcast reflects it across terminals). */
export function removeJudgeRun(sessionId: string, id: string): void {
  send({ type: "remove_judge_run", session_id: sessionId, id });
}

/** Clear a session's entire judge history. */
export function clearJudgeRuns(sessionId: string): void {
  send({ type: "clear_judge_runs", session_id: sessionId });
}

/** Overlay "Resume": continue an interrupted turn. */
export function resumeTurn(sessionId: string): void {
  send({ type: "resume_turn", session_id: sessionId });
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
// see SHIMMER_DELAY_MS); WS down → straight to `failed`. Empty is ignored.
export function addDraft(sessionId: string, text: string, attachments: Attachment[]): void {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return;
  qAdd("drafts", sessionId, trimmed, attachments);
}

// Edit a draft in place (same shape as editQueued). Clearing both fields drops it.
export function editDraft(
  sessionId: string,
  id: string,
  text: string,
  attachments: Attachment[],
): void {
  const trimmed = text.trimEnd();
  send({ type: "edit_draft", session_id: sessionId, id, text: trimmed, content: contentOf(trimmed, attachments) });
}

// Drop one draft.
export function removeDraft(sessionId: string, id: string): void {
  send({ type: "remove_draft", session_id: sessionId, id });
}

// Drop a session's whole draft list ("Clear All" on the drafts panel).
export function clearDrafts(sessionId: string): void {
  send({ type: "clear_drafts", session_id: sessionId });
}

// Activate a draft: the daemon submits it (send-or-queue) and removes it from
// drafts.
export function activateDraft(sessionId: string, id: string): void {
  send({ type: "activate_draft", session_id: sessionId, id });
}

// Send all drafts (front-to-back) — bulk "send everything" on the drafts panel.
export function activateAllDrafts(sessionId: string): void {
  send({ type: "activate_all_drafts", session_id: sessionId });
}

// Give a draft a future fire time (or reschedule one). `id` targets an existing
// draft; omit it to CREATE a scheduled draft from the composer's content (a fresh
// cmid dedups a retry). The server auto-activates it at `fireAtMs` — fires even
// with every client offline. A plain command: the server's queue patch reflects
// the new schedule (no optimistic add needed for this deliberate, rare action).
export function scheduleDraft(
  sessionId: string,
  opts: { id?: string; text?: string; attachments?: Attachment[]; fireAtMs: number; delivery?: Delivery },
): void {
  const { id, text = "", attachments = [], fireAtMs, delivery = "back" } = opts;
  send({
    type: "schedule_draft",
    session_id: sessionId,
    ...(id !== undefined ? { id } : { cmid: newCmid() }),
    text: text.trimEnd(),
    content: contentOf(text.trimEnd(), attachments),
    fire_at_ms: fireAtMs,
    delivery,
  });
}

// Strip the schedule off a draft (it stays a plain parked draft).
export function unscheduleDraft(sessionId: string, id: string): void {
  send({ type: "unschedule_draft", session_id: sessionId, id });
}

// Move a queued prompt back to drafts.
export function queuedToDraft(sessionId: string, id: string): void {
  send({ type: "queued_to_draft", session_id: sessionId, id });
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
  if (listeners.size === 0 && !socket) connect();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

// Final flush of the IndexedDB sync cache on page hide. Normal use already
// debounce-saves, so this only captures the last sub-debounce change; it's
// fire-and-forget because the unload path can't await (IndexedDB writes here are
// best-effort by spec). Failures are swallowed inside flush().
if (typeof globalThis.addEventListener === "function") {
  globalThis.addEventListener("pagehide", () => {
    for (const entry of syncClients.values()) void entry.flush();
  });
}

export function useStore(): State {
  return useSyncExternalStore(subscribe, () => state);
}

/** Subscribe to one stable slice instead of re-rendering for every unrelated
 * store mutation. `getSnapshot` keeps the previous reference when the selected
 * value is `Object.is`-equal, as required by `useSyncExternalStore`. */
export function useStoreSelector<T>(selector: (snapshot: State) => T): T {
  const selectorRef = useRef(selector);
  selectorRef.current = selector;
  const cacheRef = useRef<{ value: T } | undefined>(undefined);
  const getSnapshot = useCallback((): T => {
    const next = selectorRef.current(state);
    if (cacheRef.current && Object.is(cacheRef.current.value, next)) {
      return cacheRef.current.value;
    }
    cacheRef.current = { value: next };
    return next;
  }, []);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
