// The live TaskSummary list for one session: subagents (timeline +
// reconciliation authority), Calls (the polled ledger) and pending call
// admissions. Shared by the transcript rows, the bar and the inspector.

import { useEffect, useMemo, useSyncExternalStore } from "react";
import {
  applySubagentAuthority,
  authoritativeSubagent,
  authorityOnlySubagents,
  buildTaskSummaries,
  callAnchorsOf,
  type ReconciledBy,
  type TaskSummary,
  useAcknowledgements,
  useSessionAuthority,
} from "./backgroundTasks";
import { useCallApproval } from "./callApproval";
import { derive, pendingPermissions, type RenderItem } from "./derive";
import { cancelCall, type ManagedCallSummary, refreshCalls, useManagedCalls } from "./managedCalls";
import type { Envelope } from "./protocol";
import type { Subagent } from "./subagents";
import { cancelSubagent, useStoreSelector } from "./store";

// ---------------------------------------------------------------------------
// Stop outcomes (§6.2): Accepted -> Stopping -> the source confirms, or one of
// Rejected / Timed out (state kept) / Disconnected, each with a message.

export type StopOutcome = "stopping" | "rejected" | "timed_out" | "disconnected";

export interface StopState {
  outcome: StopOutcome;
  message: string | null;
  at: number;
}

export const STOP_TIMEOUT_MS = 30_000;

const stops = new Map<string, StopState>();
const stopListeners = new Set<() => void>();
let stopsVersion = 0;

function emitStops(): void {
  stopsVersion += 1;
  for (const listener of stopListeners) listener();
}

export function stopState(key: string): StopState | undefined {
  return stops.get(key);
}

export function setStopState(key: string, next: StopState | null): void {
  if (next === null) {
    if (!stops.delete(key)) return;
  } else {
    stops.set(key, next);
  }
  emitStops();
}

export function useStopStates(): ReadonlyMap<string, StopState> {
  useSyncExternalStore(
    (listener) => {
      stopListeners.add(listener);
      return (): void => {
        stopListeners.delete(listener);
      };
    },
    () => stopsVersion,
    () => 0,
  );
  return stops;
}

export const STOP_MESSAGE: Record<StopOutcome, string> = {
  stopping: "Stopping…",
  rejected: "Stop was rejected",
  timed_out: "Timed out, still running",
  disconnected: "Disconnected: Stop was not sent",
};

/** Ask the source to stop a task. Recording the request is not proof that it
 *  stopped: the row keeps the source state until the source confirms. */
export function requestTaskStop(
  sessionId: string,
  task: Pick<TaskSummary, "key" | "source" | "subagent" | "call">,
  send: (sessionId: string, child: string) => boolean = cancelSubagent,
): void {
  const at = Date.now();
  if (task.source === "subagent" && task.subagent) {
    if (!send(sessionId, task.subagent.id)) {
      setStopState(task.key, { outcome: "disconnected", message: null, at });
      return;
    }
    setStopState(task.key, { outcome: "stopping", message: null, at });
  } else if (task.source === "call" && task.call) {
    setStopState(task.key, { outcome: "stopping", message: null, at });
    const call = task.call;
    void cancelCall(call.placement.parent_session_id, call.call_id).then(() => {
      refreshCalls(call.placement.parent_session_id);
    }).catch((error: unknown) => {
      setStopState(task.key, {
        outcome: navigator.onLine === false ? "disconnected" : "rejected",
        message: error instanceof Error ? error.message : null,
        at: Date.now(),
      });
    });
  }
}

// ---------------------------------------------------------------------------

export interface BackgroundTasks {
  /** Transcript items with reconciled subagent content. */
  items: RenderItem[];
  tasks: TaskSummary[];
  byKey: ReadonlyMap<string, TaskSummary>;
  calls: ManagedCallSummary[];
  callsObservedAt: number | null;
  callsError: string | null;
  refreshCalls: () => void;
  /** The legacy aggregate (`SessionMeta.background_tasks`). */
  backgroundActivity: number;
}

const EMPTY_TIMELINE: Envelope[] = [];

/** Subagent snapshots, Call anchors and the worker boundaries that fence them. */
function isBackgroundEnvelope(envelope: Envelope): boolean {
  if (envelope.kind === "lifecycle") return true;
  if (envelope.kind !== "update") return false;
  const kind = envelope.update.sessionUpdate;
  return kind === "cowboy_subagent" || kind === "cowboy_call_anchor";
}

/** Subagent rows carry `Stopping` only while the source has not confirmed. */
function settleStops(tasks: readonly TaskSummary[], states: ReadonlyMap<string, StopState>, now: number): void {
  for (const task of tasks) {
    const stop = states.get(task.key);
    if (!stop) continue;
    const confirmed = task.group !== "active" && task.group !== "waiting" &&
      task.group !== "needs_you" && task.group !== "stopping";
    if (confirmed) {
      setStopState(task.key, null);
    } else if (stop.outcome === "stopping" && now - stop.at >= STOP_TIMEOUT_MS) {
      setStopState(task.key, { outcome: "timed_out", message: null, at: now });
    }
  }
}

export function useBackgroundTasks(
  sessionId: string,
  providedItems?: RenderItem[],
): BackgroundTasks {
  const timeline = useStoreSelector((snapshot) => snapshot.timelines.get(sessionId) ?? EMPTY_TIMELINE);
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  const backgroundActivity = useStoreSelector((snapshot) =>
    snapshot.sessions.find((session) => session.id === sessionId)?.background_tasks ?? 0
  );
  const lastError = useStoreSelector((snapshot) => snapshot.lastError);
  const authority = useSessionAuthority(sessionId);
  // Without the transcript's own items (the Composer bar), derive only the
  // envelopes that shape background tasks: a full derive per streamed token
  // would double the Transcript's work.
  const derived = useMemo(
    () => providedItems ?? derive(timeline.filter(isBackgroundEnvelope)),
    [providedItems, timeline],
  );
  const items = useMemo(() => applySubagentAuthority(derived, authority), [derived, authority]);
  const pending = useMemo(() => pendingPermissions(timeline), [timeline]);
  const { calls, observedAt, error, refresh } = useManagedCalls(sessionId);
  const approval = useCallApproval(sessionId);
  const acknowledged = useAcknowledgements();
  const stopStates = useStopStates();
  const unreachableMachines = useMemo(
    () => new Set(machines.filter((machine) => !machine.connected).map((machine) => machine.id)),
    [machines],
  );
  const tasks = useMemo(() => {
    const subagents: {
      subagent: Subagent;
      anchorSeq: number | null;
      reconciledBy: ReconciledBy | null;
      reconciledAt: number | null;
    }[] = items.flatMap((item) => {
      if (item.kind !== "subagent") return [];
      const resolved = authoritativeSubagent(item.subagent, authority);
      return [{
        subagent: item.subagent,
        anchorSeq: resolved.anchorSeq ?? Number(item.key),
        reconciledBy: resolved.reconciledBy,
        reconciledAt: resolved.reconciledAt,
      }];
    });
    for (const subagent of authorityOnlySubagents(items, authority)) {
      const entry = authority?.entries.get(subagent.id);
      subagents.push({
        subagent,
        anchorSeq: entry?.anchorSeq ?? null,
        reconciledBy: entry?.reconciledBy ?? null,
        reconciledAt: entry?.reconciledAt ?? null,
      });
    }
    const anchors = callAnchorsOf(items);
    const stopping = new Set(
      [...stopStates].filter(([, stop]) => stop.outcome === "stopping").map(([key]) => key),
    );
    return buildTaskSummaries({
      subagents,
      calls,
      callAnchors: new Map([...anchors].map(([id, anchor]) => [id, anchor.seq])),
      approval,
      pending,
      stopping,
      unreachableMachines,
      callsStale: error !== null && observedAt !== null,
      acknowledged,
    });
  }, [items, authority, calls, approval, pending, stopStates, unreachableMachines, error, observedAt, acknowledged]);
  // A rejected subagent Stop surfaces as a session error while it waits.
  useEffect(() => {
    if (!lastError || lastError.sessionId !== sessionId) return;
    for (const [key, stop] of stopStates) {
      if (stop.outcome === "stopping" && key.startsWith("subagent:")) {
        setStopState(key, { outcome: "rejected", message: lastError.message, at: Date.now() });
      }
    }
  }, [lastError, sessionId, stopStates]);
  useEffect(() => {
    settleStops(tasks, stopStates, Date.now());
    if (![...stopStates.values()].some((stop) => stop.outcome === "stopping")) return undefined;
    const timer = globalThis.setTimeout(() => settleStops(tasks, stopStates, Date.now()), STOP_TIMEOUT_MS);
    return () => globalThis.clearTimeout(timer);
  }, [tasks, stopStates]);
  const byKey = useMemo(() => new Map(tasks.map((task) => [task.key, task])), [tasks]);
  return {
    items,
    tasks,
    byKey,
    calls,
    callsObservedAt: observedAt,
    callsError: error,
    refreshCalls: refresh,
    backgroundActivity,
  };
}
