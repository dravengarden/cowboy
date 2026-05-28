// Single WebSocket store shared by the whole app. cowboy is the source of
// truth; this store just accumulates what it pushes. Exposed via
// useSyncExternalStore so any component re-renders on change.
//
// All clients are equal subscribers: on connect the daemon sends the session
// list + a snapshot of each session's log, then a live tail. We dedup events
// by (session_id, seq) so a reconnect snapshot overlapping the live stream is
// harmless.

import { useSyncExternalStore } from "react";
import type { Envelope, Inbound, Outbound, SessionMeta } from "./protocol";

export interface State {
  connected: boolean;
  sessions: SessionMeta[];
  // session_id → seq-ordered, deduped event log
  timelines: Map<string, Envelope[]>;
}

let state: State = { connected: false, sessions: [], timelines: new Map() };
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
    case "sessions":
      setState({ ...state, sessions: msg.sessions });
      break;
    case "snapshot": {
      let timelines = state.timelines;
      for (const env of msg.events) timelines = applyEnvelope(timelines, env);
      setState({ ...state, timelines });
      break;
    }
    case "event":
      setState({ ...state, timelines: applyEnvelope(state.timelines, msg.envelope) });
      break;
    case "error":
      console.warn("cowboy error:", msg.message);
      break;
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
