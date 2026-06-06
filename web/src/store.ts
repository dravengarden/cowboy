// Single WebSocket store shared by the whole app. cowboy is the source of
// truth; this store just accumulates what it pushes. Exposed via
// useSyncExternalStore so any component re-renders on change.
//
// All clients are equal subscribers: on connect the daemon sends the session
// list + a snapshot of each session's log, then a live tail. We dedup events
// by (session_id, seq) so a reconnect snapshot overlapping the live stream is
// harmless.

import { useSyncExternalStore } from "react";
import { type Attachment, buildContentBlocks } from "./attachments";
import type {
  ConfigOption,
  Envelope,
  Inbound,
  Outbound,
  SessionMeta,
  Status,
} from "./protocol";

/// One notification slot — the App's snackbar shows the latest. We monotonically
/// bump `seq` even on repeat messages so the UI can re-trigger the open
/// animation for the same text (e.g. user sets mode twice fast).
export interface ErrorNotice {
  seq: number;
  sessionId?: string;
  message: string;
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

/// One client-side queued prompt. Zed-style: while a session's turn is in
/// flight you stack up the next messages here instead of sending them — the
/// daemon runs each `Prompt` as a concurrent task on a single ACP connection
/// (see src/acp.rs), so two prompts mid-turn would start two overlapping turns.
/// The queue enforces strict serialization: enqueue while busy, drain exactly
/// one on each turn-end. `id` is a local monotonic key for React + edit/delete.
export interface QueuedMessage {
  id: string;
  text: string;
  /** Staged image / file attachments sent alongside the text as ACP content
   *  blocks. Empty for a plain text prompt. */
  attachments: Attachment[];
}

export interface State {
  connected: boolean;
  sessions: SessionMeta[];
  // session_id → seq-ordered, deduped event log
  timelines: Map<string, Envelope[]>;
  // session_id → agent-advertised configOptions array (mode/model/effort)
  configOptions: Map<string, ConfigOption[]>;
  // session_id → ordered prompts waiting for the current turn to finish
  queues: Map<string, QueuedMessage[]>;
  lastError?: ErrorNotice;
  // Top-of-app connection/version banner; undefined = nothing shown. Spelled
  // `| undefined` (not bare optional) so `{ ...state, banner }` can carry an
  // explicit undefined under exactOptionalPropertyTypes.
  banner?: Banner | undefined;
}

let errorSeq = 0;
let queuedSeq = 0;
let state: State = {
  connected: false,
  sessions: [],
  timelines: new Map(),
  configOptions: new Map(),
  queues: new Map(),
};
const listeners = new Set<() => void>();
let socket: WebSocket | undefined;

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

function applyEnvelope(timelines: Map<string, Envelope[]>, env: Envelope): Map<string, Envelope[]> {
  const next = new Map(timelines);
  const existing = next.get(env.session_id) ?? [];
  if (existing.some((e) => e.seq === env.seq)) return next; // dedup
  const merged = [...existing, env].sort((a, b) => a.seq - b.seq);
  next.set(env.session_id, merged);
  return next;
}

function handle(msg: Outbound): void {
  switch (msg.type) {
    case "sessions": {
      // A status broadcast is the turn-end signal: the daemon re-broadcasts the
      // session list on every set_status (src/core.rs). First reconcile the
      // in-flight guard against the rising edge into `running` (turn ended) and
      // any death, then release the next queued prompt for every session that's
      // now idle and has nothing of ours still running.
      const prevStatus = new Map<string, Status>(state.sessions.map((s) => [s.id, s.status]));
      for (const s of msg.sessions) {
        const prev = prevStatus.get(s.id);
        // Clear the in-flight guard on a true turn-end (busy → running) or on
        // death. NOT on starting → running: a revive (resuming a dead session)
        // passes through starting → running while our dispatched prompt is
        // still queued in the daemon, so clearing there would prematurely
        // release the next queued prompt and overlap turns.
        if (s.status === "running" && prev === "busy") inFlight.delete(s.id);
        if (s.status === "exited" || s.status === "crashed") inFlight.delete(s.id);
      }
      let queues = state.queues;
      const toSend: QueuedMessage[] = [];
      const toSendIds: string[] = [];
      for (const s of msg.sessions) {
        if (!canDispatch(s.id, s.status)) continue;
        const q = queues.get(s.id);
        const head = q?.[0];
        if (!q || !head) continue;
        if (queues === state.queues) queues = new Map(state.queues);
        const rest = q.slice(1);
        if (rest.length > 0) queues.set(s.id, rest);
        else queues.delete(s.id);
        inFlight.add(s.id); // claim the slot now so a same-tick re-broadcast can't double-send
        toSend.push(head);
        toSendIds.push(s.id);
      }
      setState({ ...state, sessions: msg.sessions, queues });
      for (const [i, head] of toSend.entries()) {
        const sessionId = toSendIds[i];
        if (sessionId) send(promptCommand(sessionId, head.text, head.attachments));
      }
      break;
    }
    case "snapshot": {
      let timelines = state.timelines;
      for (const env of msg.events) timelines = applyEnvelope(timelines, env);
      setState({ ...state, timelines });
      break;
    }
    case "event":
      setState({ ...state, timelines: applyEnvelope(state.timelines, msg.envelope) });
      break;
    case "config_options": {
      const next = new Map(state.configOptions);
      next.set(msg.session_id, msg.options);
      setState({ ...state, configOptions: next });
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

// The update overlay's reload action (fired when its countdown elapses): a hard
// reload. index.html ships `no-cache` (always revalidated) and the hashed assets
// are immutable, so the next load pulls the new bundle cleanly.
export function applyUpdate(): void {
  globalThis.location.reload();
}

// First thing on every (re)connect: ask the daemon for its build id. On the
// very first probe we only record the baseline; thereafter a changed id means
// the server was redeployed under us, so we raise the (sticky) update banner.
// A failed probe is a no-op — we simply try again on the next reconnect.
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
  if (version !== knownVersion) setBanner({ kind: "update" });
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

export function send(cmd: Inbound): void {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(cmd));
  }
}

// --- Queued prompts ---------------------------------------------------------

// session_ids with a prompt we've dispatched whose turn-end we haven't observed
// yet. The daemon runs each Prompt as a concurrent task on one ACP connection
// (src/acp.rs), so a second dispatch before turn-end would start an overlapping
// turn. This guard makes the queue strictly serial *regardless of broadcast
// lag*: we only dispatch when a session is running AND nothing of ours is in
// flight, and the flag is cleared on the turn-end (→running) edge or on death
// (see the "sessions" handler). It's module state, not React state — flipping it
// must never trigger a render, and it's purely a dispatch gate.
const inFlight = new Set<string>();

function nextQueuedId(): string {
  queuedSeq += 1;
  return `q${queuedSeq}`;
}

function canDispatch(sessionId: string, status: Status): boolean {
  // "running" is the normal idle-ready state. "exited"/"crashed" are also
  // dispatchable: sending to a dead session is what resumes it — the daemon
  // revives the agent (session/load) on receiving the prompt. Without this a
  // prompt to a dead session would sit in the queue forever (nothing would
  // ever flip it to "running" first).
  const resumable = status === "running" || status === "exited" || status === "crashed";
  return resumable && !inFlight.has(sessionId);
}

// Build the WS prompt command for a turn. With attachments, send the ACP
// `content` block array (images / embedded file resources + a trailing text
// block); without, the legacy text-only shape the daemon wraps in one Text
// block. See attachments.ts + src/core.rs `Inbound::Prompt`.
function promptCommand(sessionId: string, text: string, attachments: Attachment[]): Inbound {
  const content = buildContentBlocks(text, attachments);
  if (content) return { type: "prompt", session_id: sessionId, content };
  return { type: "prompt", session_id: sessionId, text };
}

function dispatchPrompt(sessionId: string, text: string, attachments: Attachment[]): void {
  inFlight.add(sessionId);
  send(promptCommand(sessionId, text, attachments));
}

function enqueue(sessionId: string, text: string, attachments: Attachment[]): void {
  const next = new Map(state.queues);
  const q = next.get(sessionId) ?? [];
  next.set(sessionId, [...q, { id: nextQueuedId(), text, attachments }]);
  setState({ ...state, queues: next });
}

// The single entry point the composer calls to send a user prompt. Sends
// straight through when the session can take a turn right now; otherwise stacks
// it on the queue to drain on the next turn-end. A prompt with at least one
// attachment is valid even with empty text (an image speaks for itself);
// otherwise empty text is ignored.
export function submitPrompt(
  sessionId: string,
  status: Status,
  text: string,
  attachments: Attachment[] = [],
): void {
  const trimmed = text.trimEnd();
  if (!trimmed.trim() && attachments.length === 0) return;
  if (canDispatch(sessionId, status)) dispatchPrompt(sessionId, trimmed, attachments);
  else enqueue(sessionId, trimmed, attachments);
}

// "Send now" on a queued row. If the session can take a turn this instant, send
// it and drop it from the queue; otherwise move it to the front so it's the
// next one drained — the daemon can't run a concurrent turn, so the honest best
// "now" while busy is "first after this turn".
export function requestSendQueued(sessionId: string, status: Status, id: string): void {
  if (canDispatch(sessionId, status)) {
    const item = state.queues.get(sessionId)?.find((m) => m.id === id);
    if (!item) return;
    removeQueued(sessionId, id);
    dispatchPrompt(sessionId, item.text, item.attachments);
  } else {
    promoteQueued(sessionId, id);
  }
}

// "Force push" a queued row: interrupt the running turn and make this prompt
// the one that runs next. Cancel is ASYNC — the daemon's turn doesn't end the
// instant we send it, so we must NOT dispatch here: a dispatch racing the
// still-cancelling turn would start an overlapping turn and break the inFlight
// serialization (see the `inFlight` note above). Instead promote this to the
// front and send Cancel; the busy → running turn-end edge then drains the front
// item (this prompt) via the "sessions" handler. If the session can already
// take a turn there's nothing to interrupt — fall back to a plain send.
export function forcePushQueued(sessionId: string, status: Status, id: string): void {
  if (canDispatch(sessionId, status)) {
    requestSendQueued(sessionId, status, id);
    return;
  }
  promoteQueued(sessionId, id);
  send({ type: "cancel", session_id: sessionId });
}

// Edit a queued prompt in place. Clearing the text removes the entry.
export function editQueued(sessionId: string, id: string, text: string): void {
  const trimmed = text.trimEnd();
  const q = state.queues.get(sessionId);
  if (!q) return;
  // Clearing the text removes the entry only when it carries no attachments —
  // an attachment-only prompt (e.g. just a pasted screenshot) stays valid.
  if (!trimmed.trim()) {
    const existing = q.find((m) => m.id === id);
    if (!existing || existing.attachments.length === 0) {
      removeQueued(sessionId, id);
      return;
    }
  }
  const next = new Map(state.queues);
  next.set(sessionId, q.map((m) => (m.id === id ? { ...m, text: trimmed } : m)));
  setState({ ...state, queues: next });
}

// Drop one queued prompt.
export function removeQueued(sessionId: string, id: string): void {
  const q = state.queues.get(sessionId);
  if (!q) return;
  const filtered = q.filter((m) => m.id !== id);
  const next = new Map(state.queues);
  if (filtered.length > 0) next.set(sessionId, filtered);
  else next.delete(sessionId);
  setState({ ...state, queues: next });
}

// Drop a session's whole queue (the "Clear All" header action).
export function clearQueue(sessionId: string): void {
  if (!state.queues.has(sessionId)) return;
  const next = new Map(state.queues);
  next.delete(sessionId);
  setState({ ...state, queues: next });
}

// Move a queued prompt to the front so it's the next one drained.
function promoteQueued(sessionId: string, id: string): void {
  const q = state.queues.get(sessionId);
  if (!q || q[0]?.id === id) return; // already at front (or unknown)
  const item = q.find((m) => m.id === id);
  if (!item) return;
  const next = new Map(state.queues);
  next.set(sessionId, [item, ...q.filter((m) => m.id !== id)]);
  setState({ ...state, queues: next });
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
