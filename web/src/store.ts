// Single WebSocket store shared by the whole app. cowboy is the source of
// truth; this store just accumulates what it pushes. Exposed via
// useSyncExternalStore so any component re-renders on change.
//
// All clients are equal subscribers: on connect the daemon sends the session
// list + a snapshot of each session's log, then a live tail. We dedup events
// by (session_id, seq) so a reconnect snapshot overlapping the live stream is
// harmless.

import { useSyncExternalStore } from "react";
import { type ArgsOf, createClient, type Mutation, type Mutators, snapshotPatch } from "./_sync/mod.ts";
import { type Attachment, blocksToAttachments, buildContentBlocks } from "./attachments";
import { pruneDrafts } from "./draftStore";
import { fireAlert } from "./turnNotify";
import type {
  ConfigOption,
  ContentBlock,
  Envelope,
  Inbound,
  Outbound,
  SessionMeta,
  WireQueued,
} from "./protocol";

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

/// Top-of-app connection banner. Purely client-side — NOT part of the WS
/// protocol; the store drives it off the socket lifecycle plus a build-version
/// probe:
///   - "down"        — reconnect has failed RECONNECT_BANNER_THRESHOLD times in
///                     a row; red, so the user knows the live stream is stale
///                     while we keep retrying underneath.
///   - "reconnected" — the socket came back after a "down" banner was shown;
///                     green, auto-dismissed after RECONNECTED_DISMISS_MS.
///   - "update"      — the post-reconnect version probe saw a new server build;
///                     blue, sticky, click reloads to pull the new bundle.
export type BannerKind = "down" | "reconnected" | "update";
export interface Banner {
  kind: BannerKind;
}

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
  pagination: Map<string, { reachedStart: boolean; loadingOlder: boolean; nextPage: number }>;
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
  // session_id → LOCAL optimistic rows awaiting daemon confirmation (carry a
  // `status`). Purely client-side — NOT synced, NOT persisted. Reconciled away
  // the instant their `cmid` shows up server-side (so an optimistic row and its
  // confirmed twin never coexist). `drafts`+`queue` reconcile via the `queues`
  // broadcast; `messages` (chat send) via the user-echo Envelope's cmid.
  optimisticDrafts: Map<string, QueuedMessage[]>;
  optimisticQueue: Map<string, QueuedMessage[]>;
  optimisticMessages: Map<string, QueuedMessage[]>;
  lastError?: ErrorNotice;
  // Top-of-app connection/version banner; undefined = nothing shown. Spelled
  // `| undefined` (not bare optional) so `{ ...state, banner }` can carry an
  // explicit undefined under exactOptionalPropertyTypes.
  banner?: Banner | undefined;
  // session_id → title from the "title" sync state (@shared-utils/sync). Source
  // of title truth on the client: overlaid onto SessionMeta.title by
  // `deriveSessions`, so an optimistic rename shows instantly and every terminal
  // converges on the arbiter's `sync_patch`. Ids absent here fall back to the
  // broadcast SessionMeta.title.
  titleOverrides: Record<string, string>;
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
  optimisticDrafts: new Map(),
  optimisticQueue: new Map(),
  optimisticMessages: new Map(),
  titleOverrides: {},
};
const listeners = new Set<() => void>();
let socket: WebSocket | undefined;
// The session the user currently has open. Remembered so every (re)connect can
// re-assert it to the daemon (revive-on-open), recovering the agent after a
// daemon restart we reconnected across. See openSession + connect's onopen.
let openedSessionId: string | undefined;

// --- Reconnect + version bookkeeping ----------------------------------------

// Surface the red "down" banner once this many consecutive (re)connect cycles
// have failed — a single dropped frame that recovers on the first retry stays
// silent; only a real outage raises the banner.
const RECONNECT_BANNER_THRESHOLD = 2;
// Cap the exponential backoff so a long outage doesn't hammer the daemon, while
// a brief blip still recovers within a second.
const RECONNECT_BACKOFF_MAX_MS = 15_000;
// How long the green "reconnected" flash lingers before auto-dismissing.
const RECONNECTED_DISMISS_MS = 4_000;

// Consecutive failed (re)connect cycles; reset to 0 on a successful open.
let reconnectAttempts = 0;
// Whether the current outage actually surfaced the red banner — so the eventual
// reopen only flashes green for outages the user was told about, not for a blip
// that never crossed the threshold.
let outageSurfaced = false;
let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
let reconnectedTimer: ReturnType<typeof setTimeout> | undefined;

// The server build id this tab first loaded against. `/version` returns the
// embedded index.html's content hash, which changes on every redeploy that
// alters the shipped bundle and stays stable otherwise. We capture it once,
// then re-probe after each successful reconnect: a mismatch means the daemon
// was redeployed under a now-stale tab → raise the blue "update" banner.
let knownVersion: string | undefined;

function emit(): void {
  for (const l of listeners) l();
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
    attachments: blocksToAttachments(m.content),
    // Carried so an optimistic row can reconcile against its confirmed twin by
    // id. `?? undefined` keeps `exactOptionalPropertyTypes` happy.
    ...(m.cmid !== undefined && { cmid: m.cmid }),
  }));
}

function applyEnvelope(timelines: Map<string, Envelope[]>, env: Envelope): Map<string, Envelope[]> {
  const next = new Map(timelines);
  const existing = next.get(env.session_id) ?? [];
  if (existing.some((e) => e.seq === env.seq)) return next; // dedup
  const merged = [...existing, env].sort((a, b) => a.seq - b.seq);
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
  const seen = new Set(existing.map((e) => e.seq));
  const fresh = events.filter((e) => !seen.has(e.seq));
  if (fresh.length === 0 && existing.length > 0) return next;
  const merged = [...existing, ...fresh].sort((a, b) => a.seq - b.seq);
  next.set(sessionId, merged);
  return next;
}

// History page size — MUST match the server's HISTORY_PAGE (src/core.rs). Pages
// are seq-aligned so each has a stable, cacheable URL.
const HISTORY_PAGE = 200;

// Fetch the next OLDER page of a session's history and prepend it. Pages come
// from the immutable HTTP route (GET /api/history/:id/:page) so a re-fetch
// (scroll back, reload, post-recycle) is a cache hit — zero network. One fetch
// at a time per session; no-op once the window already reaches the first event.
//
// VERSION-SCOPED url (`?v=<build>`): the cache is `immutable`, so without this a
// redeploy could keep serving pages cached under the OLD build (if a future
// version ever changes the event shape, that's stale/incompatible). Keying the
// url on the build id means a new version's pages are fresh fetches, while
// reloads of the SAME build stay cache hits. (`knownVersion` is the id the tab
// loaded against; until the first /version probe lands it's a harmless "0".)
export async function loadOlder(sessionId: string): Promise<void> {
  const pg = state.pagination.get(sessionId);
  if (!pg || pg.reachedStart || pg.loadingOlder || pg.nextPage < 0) return;
  const page = pg.nextPage;
  setPagination(sessionId, { ...pg, loadingOlder: true });
  try {
    const res = await fetch(
      `/api/history/${encodeURIComponent(sessionId)}/${String(page)}?v=${encodeURIComponent(knownVersion ?? "0")}`,
    );
    if (!res.ok) {
      setPagination(sessionId, { ...pg, loadingOlder: false });
      return;
    }
    const data = (await res.json()) as { events: Envelope[] };
    setState({ ...state, timelines: mergeEvents(state.timelines, sessionId, data.events) });
    // Always step to the next OLDER page (don't recompute from the oldest seq —
    // a gap at a boundary would re-request the same page forever). Page 0 was
    // the last → reached start.
    const nextPage = page - 1;
    setPagination(sessionId, { reachedStart: nextPage < 0, loadingOlder: false, nextPage });
  } catch {
    setPagination(sessionId, { ...pg, loadingOlder: false });
  }
}

function setPagination(
  sessionId: string,
  value: { reachedStart: boolean; loadingOlder: boolean; nextPage: number },
): void {
  const pagination = new Map(state.pagination);
  pagination.set(sessionId, value);
  setState({ ...state, pagination });
}

function handle(msg: Outbound): void {
  switch (msg.type) {
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
    case "queues": {
      // Authoritative queue + drafts for one session — replace ours wholesale.
      const queues = new Map(state.queues);
      const drafts = new Map(state.drafts);
      const q = fromWire(msg.queue);
      const d = fromWire(msg.drafts);
      if (q.length > 0) queues.set(msg.session_id, q);
      else queues.delete(msg.session_id);
      if (d.length > 0) drafts.set(msg.session_id, d);
      else drafts.delete(msg.session_id);
      // Reconcile: any cmid now present server-side (queue OR drafts) is
      // CONFIRMED → drop the matching optimistic row from ALL three overlays
      // (so even a wrong idle/busy guess self-corrects — no duplicate). Match by
      // cmid only, never text; clears its timers too.
      const sid = msg.session_id;
      const serverCmids = new Set([...d, ...q].map((m) => m.cmid).filter(Boolean));
      setState({
        ...state,
        queues,
        drafts,
        optimisticDrafts: reconcileOptimistic(state.optimisticDrafts, sid, serverCmids),
        optimisticQueue: reconcileOptimistic(state.optimisticQueue, sid, serverCmids),
        optimisticMessages: reconcileOptimistic(state.optimisticMessages, sid, serverCmids),
      });
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
            nextPage: msg.events.length
              ? Math.floor(((msg.events[0]?.seq ?? 0) - 1) / HISTORY_PAGE)
              : -1,
          });
      setState({ ...state, timelines, hydrated, pagination });
      break;
    }
    case "event": {
      const env = msg.envelope;
      // Attention alert — ONLY the two events that actually need the user: a
      // finished turn (done) and a permission request (confirm). These are LIVE
      // events; snapshot/history replays go through `case "snapshot"`, so past
      // turns never re-ding. `fireAlert` no-ops when the setting is off.
      if (env.kind === "turn_end" || env.kind === "permission_request") fireAlert();
      // The dispatched prompt's user-echo carries the originating client's cmid
      // → CONFIRMS the optimistic chat bubble (and any wrong-guessed draft/queue
      // row): drop it from all overlays the instant the real one lands.
      const reconcile = env.cmid !== undefined;
      const cmids = new Set<string | undefined>(env.cmid !== undefined ? [env.cmid] : []);
      setState({
        ...state,
        timelines: applyEnvelope(state.timelines, env),
        ...(reconcile && {
          optimisticMessages: reconcileOptimistic(state.optimisticMessages, env.session_id, cmids),
          optimisticDrafts: reconcileOptimistic(state.optimisticDrafts, env.session_id, cmids),
          optimisticQueue: reconcileOptimistic(state.optimisticQueue, env.session_id, cmids),
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
      // (keeps the highest version, drops confirmed pending, rebases the rest)
      // and re-derive the session list. Unknown state → ignored (forward-compat).
      syncClients.get(msg.state)?.applyPatch(msg.version, msg.value, msg.confirmed);
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
      break;
    }
  }
}

function setBanner(banner: Banner | undefined): void {
  setState({ ...state, banner });
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

// The update overlay's reload action (fired when its countdown elapses): a hard
// reload. index.html ships `no-cache` (always revalidated) and the hashed assets
// are immutable, so the next load pulls the new bundle cleanly.
export function applyUpdate(): void {
  globalThis.location.reload();
}

// First thing on every (re)connect: ask the daemon for its build id. On the
// very first probe we only record the baseline; thereafter a changed id means
// the server was redeployed under us. We AUTO-RELOAD in that case rather than
// just showing a banner: a redeploy restarts the daemon, the PWA reconnects over
// it, but an installed PWA keeps its OLD cached JS until a full reload — a WS
// reconnect is not enough. The banner was routinely ignored, so every shipped
// web change read as "no effect". Auto-reloading is safe here: the conversation
// is server-persisted and the composer text is in the per-session draft store
// (localStorage), so the reload restores everything. `/version` is the embedded
// index.html content hash (stable per build), so this fires exactly once per
// deploy and the fresh load re-baselines `knownVersion` (no reload loop). A
// failed probe is a no-op — we retry on the next reconnect.
async function probeVersion(): Promise<void> {
  let version: string;
  try {
    const res = await fetch("/version", { cache: "no-store" });
    if (!res.ok) return;
    ({ version } = (await res.json()) as { version: string });
  } catch {
    return;
  }
  if (knownVersion === undefined) {
    knownVersion = version;
    return;
  }
  if (version !== knownVersion) {
    // Brief defer so the just-arrived reconnect snapshot settles before the
    // reload (avoids reloading mid-handshake); state persists either way.
    setBanner({ kind: "update" });
    globalThis.setTimeout(() => applyUpdate(), 600);
  }
}

function scheduleReconnect(): void {
  if (reconnectTimer) return; // one pending attempt at a time
  const delay = Math.min(
    RECONNECT_BACKOFF_MAX_MS,
    1000 * 2 ** Math.max(0, reconnectAttempts - 1),
  );
  reconnectTimer = setTimeout(() => {
    reconnectTimer = undefined;
    connect();
  }, delay);
}

function connect(): void {
  const proto = globalThis.location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${proto}//${globalThis.location.host}/ws`);
  socket = ws;
  ws.onopen = (): void => {
    const recovered = outageSurfaced;
    reconnectAttempts = 0;
    outageSurfaced = false;
    // Recovered from a surfaced outage → flash green, but never stomp a sticky
    // blue update banner (it outranks everything). The async version probe may
    // replace the green with blue moments later.
    let banner = state.banner;
    if (recovered && banner?.kind !== "update") banner = { kind: "reconnected" };
    setState({ ...state, connected: true, banner });
    if (banner?.kind === "reconnected") {
      if (reconnectedTimer) clearTimeout(reconnectedTimer);
      reconnectedTimer = setTimeout(() => {
        reconnectedTimer = undefined;
        // Only clear if still green — don't stomp an update banner the probe
        // raised in the meantime.
        if (state.banner?.kind === "reconnected") setBanner(undefined);
      }, RECONNECTED_DISMISS_MS);
    }
    void probeVersion();
    // Re-assert the open session so the daemon revives its agent if it died
    // with a restart we just reconnected across (revive-on-open, design §7).
    // Idempotent server-side when the agent is still alive.
    if (openedSessionId) {
      send({ type: "open_session", session_id: openedSessionId });
    }
    // Re-send sync mutations the arbiter never confirmed (sent while the socket
    // was down). The mutation id makes the daemon idempotent; the resync
    // `sync_patch` that follows drops them from pending once confirmed.
    for (const entry of syncClients.values()) entry.resend();
  };
  ws.onmessage = (e: MessageEvent<string>): void => {
    try {
      handle(JSON.parse(e.data) as Outbound);
    } catch (err) {
      console.warn("bad message", err);
    }
  };
  ws.onclose = (): void => {
    reconnectAttempts += 1;
    // Raise the red banner once retries have failed past the threshold, unless
    // a sticky update banner is already up (that one outranks the outage).
    let banner = state.banner;
    if (reconnectAttempts >= RECONNECT_BANNER_THRESHOLD && banner?.kind !== "update") {
      outageSurfaced = true;
      banner = { kind: "down" };
    }
    setState({ ...state, connected: false, banner });
    scheduleReconnect();
  };
  ws.onerror = (): void => ws.close();
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

// --- Optimistic sends (local, never synced) ---------------------------------
// Show a staged/sent item INSTANTLY, reconciled away by cmid when the daemon
// echoes it back. THREE overlays, one machinery:
//   - "drafts"   → server `queues` broadcast echoes the cmid
//   - "queue"    → same `queues` broadcast (busy submit)
//   - "messages" → the dispatched prompt's user-echo Envelope carries the cmid
//                  (chat send, idle submit)
type OptKind = "drafts" | "queue" | "messages";

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
  applyPatch: (version: number, value: unknown, confirmed: string[]) => void;
  resend: () => void;
}
const syncClients = new Map<string, SyncEntry>();
const syncBase = newCmid(); // namespaces mutation ids across states + this tab

/** Wire one synced state to the generic channel: a lib client + optimistic
 *  mutate (instant local + send) + patch fold + resend-on-reconnect. */
function registerSync<T, M extends Mutators<T>>(
  syncState: string,
  mutators: M,
  initial: T,
): { view: () => T; mutate: <K extends keyof M & string>(name: K, args: ArgsOf<T, M, K>) => void } {
  const client = createClient<T, M>({
    clientId: `${syncBase}:${syncState}`,
    mutators,
    initial: { version: 0, value: initial },
  });
  const sendMutation = (m: Mutation): void => {
    send({ type: "sync", state: syncState, id: m.id, name: m.name, args: m.args });
  };
  syncClients.set(syncState, {
    applyPatch: (version, value, confirmed): void => {
      client.applyPatch(snapshotPatch(version, value as T, confirmed));
      commitSessions();
    },
    resend: (): void => {
      for (const m of client.pending()) sendMutation(m);
    },
  });
  return {
    view: (): T => client.view(),
    mutate: (name, args): void => {
      const m = client.mutate(name, args);
      commitSessions();
      sendMutation(m);
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

function optGet(kind: OptKind): Map<string, QueuedMessage[]> {
  return kind === "drafts"
    ? state.optimisticDrafts
    : kind === "queue"
      ? state.optimisticQueue
      : state.optimisticMessages;
}
function optCommit(kind: OptKind, map: Map<string, QueuedMessage[]>): void {
  if (kind === "drafts") setState({ ...state, optimisticDrafts: map });
  else if (kind === "queue") setState({ ...state, optimisticQueue: map });
  else setState({ ...state, optimisticMessages: map });
}

// cmid → its pending/timeout timers, so reconcile/retry can clear them.
const optTimers = new Map<string, { shimmer?: ReturnType<typeof setTimeout>; fail?: ReturnType<typeof setTimeout> }>();
function clearOptTimers(cmid: string): void {
  const t = optTimers.get(cmid);
  if (t?.shimmer) clearTimeout(t.shimmer);
  if (t?.fail) clearTimeout(t.fail);
  optTimers.delete(cmid);
}

/** Mutate one optimistic row's status in place (immutably), or drop it. */
function patchOptimistic(
  kind: OptKind,
  sessionId: string,
  cmid: string,
  patch: ((m: QueuedMessage) => QueuedMessage) | "drop",
): void {
  const list = optGet(kind).get(sessionId);
  if (!list) return;
  const next = patch === "drop"
    ? list.filter((m) => m.cmid !== cmid)
    : list.map((m) => (m.cmid === cmid ? patch(m) : m));
  const map = new Map(optGet(kind));
  if (next.length > 0) map.set(sessionId, next);
  else map.delete(sessionId);
  optCommit(kind, map);
}

/** Append a fresh optimistic row to an overlay. */
function addOptimistic(kind: OptKind, sessionId: string, row: QueuedMessage): void {
  const map = new Map(optGet(kind));
  map.set(sessionId, [...(map.get(sessionId) ?? []), row]);
  optCommit(kind, map);
}

/** Arm the pending→sending (shimmer) and →failed timers for a sending row. */
function armOptTimers(kind: OptKind, sessionId: string, cmid: string): void {
  clearOptTimers(cmid);
  optTimers.set(cmid, {
    shimmer: setTimeout(() => {
      patchOptimistic(kind, sessionId, cmid, (m) =>
        m.status === "pending" ? { ...m, status: "sending" } : m);
    }, SHIMMER_DELAY_MS),
    fail: setTimeout(() => {
      patchOptimistic(kind, sessionId, cmid, (m) =>
        m.status === "failed" ? m : { ...m, status: "failed" });
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

/** Retry a failed optimistic row from THIS device (idempotent — the daemon
 *  dedupes by cmid). Back to `pending`, re-armed. Shared by all three kinds. */
function retryOptimistic(kind: OptKind, sessionId: string, cmid: string): void {
  const row = optGet(kind).get(sessionId)?.find((m) => m.cmid === cmid);
  if (!row) return;
  const cmd: Inbound = kind === "drafts"
    ? { type: "add_draft", session_id: sessionId, text: row.text, content: contentOf(row.text, row.attachments), cmid }
    : { type: "submit", session_id: sessionId, text: row.text, content: contentOf(row.text, row.attachments), cmid };
  const sent = send(cmd);
  patchOptimistic(kind, sessionId, cmid, (m) => ({ ...m, status: sent ? "pending" : "failed" }));
  if (sent) armOptTimers(kind, sessionId, cmid);
}

/** Discard a (usually failed) optimistic row locally — it never reached the
 *  daemon, so there's nothing server-side to remove. */
function discardOptimistic(kind: OptKind, sessionId: string, cmid: string): void {
  clearOptTimers(cmid);
  patchOptimistic(kind, sessionId, cmid, "drop");
}

/** Mint a cmid, fire the command, and show the optimistic row in one shot —
 *  shared by addDraft + submitPrompt. WS open → `pending`; WS down → `failed`. */
function optimisticSend(
  kind: OptKind,
  sessionId: string,
  text: string,
  attachments: Attachment[],
  cmd: (cmid: string) => Inbound,
): void {
  const cmid = newCmid();
  const sent = send(cmd(cmid));
  addOptimistic(kind, sessionId, {
    id: `opt-${cmid}`,
    text,
    attachments,
    cmid,
    status: sent ? "pending" : "failed",
  });
  if (sent) armOptTimers(kind, sessionId, cmid);
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
  const queueEmpty = (state.queues.get(sessionId)?.length ?? 0) === 0
    && (state.optimisticQueue.get(sessionId)?.length ?? 0) === 0;
  const kind: OptKind = dispatchable && queueEmpty ? "messages" : "queue";
  optimisticSend(kind, sessionId, trimmed, attachments, (cmid) => ({
    type: "submit",
    session_id: sessionId,
    text: trimmed,
    content: contentOf(trimmed, attachments),
    cmid,
  }));
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
  optimisticSend("drafts", sessionId, trimmed, attachments, (cmid) => ({
    type: "add_draft",
    session_id: sessionId,
    text: trimmed,
    content: contentOf(trimmed, attachments),
    cmid,
  }));
}

/** Retry / discard a failed optimistic row from THIS device — re-exported for
 *  the row UI. Retry is idempotent (daemon dedupes by cmid). */
export { retryOptimistic, discardOptimistic };

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

export function useStore(): State {
  return useSyncExternalStore(subscribe, () => state);
}
