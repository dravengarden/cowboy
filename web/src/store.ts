// Single WebSocket store shared by the whole app. cowboy is the source of
// truth; this store just accumulates what it pushes. Exposed via
// useSyncExternalStore so any component re-renders on change.
//
// All clients are equal subscribers: on connect the daemon sends the session
// list + a snapshot of each session's log, then a live tail. We dedup events
// by (session_id, seq) so a reconnect snapshot overlapping the live stream is
// harmless.

import { useSyncExternalStore } from "react";
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

/// One client-side queued prompt. Zed-style: while a session's turn is in
/// flight you stack up the next messages here instead of sending them — the
/// daemon runs each `Prompt` as a concurrent task on a single ACP connection
/// (see src/acp.rs), so two prompts mid-turn would start two overlapping turns.
/// The queue enforces strict serialization: enqueue while busy, drain exactly
/// one on each turn-end. `id` is a local monotonic key for React + edit/delete.
export interface QueuedMessage {
  id: string;
  text: string;
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
      const toSend: { sessionId: string; text: string }[] = [];
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
        toSend.push({ sessionId: s.id, text: head.text });
      }
      setState({ ...state, sessions: msg.sessions, queues });
      for (const p of toSend) send({ type: "prompt", session_id: p.sessionId, text: p.text });
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

function connect(): void {
  const proto = globalThis.location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${proto}//${globalThis.location.host}/ws`);
  socket = ws;
  ws.onopen = (): void => setState({ ...state, connected: true });
  ws.onmessage = (e: MessageEvent<string>): void => {
    try {
      handle(JSON.parse(e.data) as Outbound);
    } catch (err) {
      console.warn("bad message", err);
    }
  };
  ws.onclose = (): void => {
    setState({ ...state, connected: false });
    // Reconnect with a fresh snapshot after a short delay.
    setTimeout(connect, 1000);
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

function dispatchPrompt(sessionId: string, text: string): void {
  inFlight.add(sessionId);
  send({ type: "prompt", session_id: sessionId, text });
}

function enqueue(sessionId: string, text: string): void {
  const next = new Map(state.queues);
  const q = next.get(sessionId) ?? [];
  next.set(sessionId, [...q, { id: nextQueuedId(), text }]);
  setState({ ...state, queues: next });
}

// The single entry point the composer calls to send a user prompt. Sends
// straight through when the session can take a turn right now; otherwise stacks
// it on the queue to drain on the next turn-end. Empty text is ignored.
export function submitPrompt(sessionId: string, status: Status, text: string): void {
  const trimmed = text.trimEnd();
  if (!trimmed.trim()) return;
  if (canDispatch(sessionId, status)) dispatchPrompt(sessionId, trimmed);
  else enqueue(sessionId, trimmed);
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
    dispatchPrompt(sessionId, item.text);
  } else {
    promoteQueued(sessionId, id);
  }
}

// Edit a queued prompt in place. Clearing the text removes the entry.
export function editQueued(sessionId: string, id: string, text: string): void {
  const trimmed = text.trimEnd();
  if (!trimmed.trim()) {
    removeQueued(sessionId, id);
    return;
  }
  const q = state.queues.get(sessionId);
  if (!q) return;
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
