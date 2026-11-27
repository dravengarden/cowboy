// Shared Stop confirmation guard binding an open confirmation dialog to the
// exact session, task identity, latest observed execution snapshot, and live
// socket connection generation so a resumed subagent run or newer worker can
// never be stopped by a stale dialog.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { authoritativeSubagent, sessionAuthority, type TaskSummary } from "../backgroundTasks";
import { connectionGeneration, subscribeConnectionGeneration } from "../clientConnection";
import { derive } from "../derive";
import { cachedCallSummary, callActive } from "../managedCalls";
import { canonicalTimeline, useStoreSelector } from "../store";
import { subagentTerminal } from "../subagents";
import { requestTaskStop, STOP_MESSAGE, type StopState, useStopStates } from "../useBackgroundTasks";

export function stopGrantKey(
  sessionId: string,
  task: TaskSummary,
  connected: boolean,
  disconnectEpoch: number,
  connGen = 0,
): string | null {
  if (!task.canStop) return null;
  const connPart = `${connected ? "1" : "0"}:${String(disconnectEpoch)}:${String(connGen)}`;
  if (task.source === "subagent" && task.subagent) {
    const sub = task.subagent;
    return `${sessionId}\u0000subagent:${sub.id}\u0000${sub.incarnation ?? ""}\u0000rev:${String(sub.revision)}\u0000seq:${String(sub.observedSeq ?? "")}\u0000term:${String(sub.terminalSeq ?? "")}\u0000${connPart}`;
  }
  if (task.source === "call" && task.call) {
    return `${sessionId}\u0000call:${task.call.call_id}\u0000${task.call.state}\u0000${String(task.call.updated_at_ms)}\u0000${connPart}`;
  }
  return null;
}

/** Revalidate a captured task synchronously against the canonical timeline,
 *  subagent authority, and call cache immediately before dispatching Stop. */
export function matchesLiveCanonicalTask(sessionId: string, task: TaskSummary): boolean {
  if (!task.canStop) return false;
  if (task.source === "subagent" && task.subagent) {
    const timeline = canonicalTimeline(sessionId);
    const authority = sessionAuthority(sessionId);
    if (timeline === undefined && authority === undefined) return true;
    const tlItem = timeline
      ? derive(timeline).find(
        (item) => item.kind === "subagent" && item.subagent.id === task.subagent!.id,
      )
      : undefined;
    const tlSub = tlItem?.kind === "subagent" ? tlItem.subagent : null;
    const liveSub = tlSub
      ? authoritativeSubagent(tlSub, authority).subagent
      : authority?.entries.get(task.subagent.id)?.subagent ?? null;
    if (!liveSub || subagentTerminal(liveSub) || liveSub.unavailable || !liveSub.cancellable) {
      return false;
    }
    return (
      liveSub.incarnation === task.subagent.incarnation &&
      liveSub.revision === task.subagent.revision &&
      liveSub.observedSeq === task.subagent.observedSeq &&
      liveSub.terminalSeq === task.subagent.terminalSeq
    );
  }
  if (task.source === "call" && task.call) {
    const liveCall = cachedCallSummary(sessionId, task.call.call_id);
    if (!liveCall) return true;
    return (
      callActive(liveCall.state) &&
      liveCall.state === task.call.state &&
      liveCall.updated_at_ms === task.call.updated_at_ms
    );
  }
  return false;
}

export function useBoundStopConfirm(
  sessionId: string,
  task: TaskSummary,
  options?: {
    stop?: StopState | undefined;
    stopSender?: ((sessionId: string, child: string) => boolean) | undefined;
  },
) {
  const stops = useStopStates();
  const stop = options?.stop !== undefined ? options.stop : stops.get(task.key);
  const connected = useStoreSelector((snapshot) => snapshot.connected);
  const connGen = useSyncExternalStore(
    subscribeConnectionGeneration,
    connectionGeneration,
    connectionGeneration,
  );
  const prevConnectedRef = useRef(connected);
  const disconnectEpochRef = useRef(0);
  if (prevConnectedRef.current && !connected) {
    disconnectEpochRef.current += 1;
  }
  prevConnectedRef.current = connected;

  const currentGrant = stopGrantKey(sessionId, task, connected, disconnectEpochRef.current, connGen);
  const [pending, setPending] = useState<{
    sessionId: string;
    task: TaskSummary;
    grant: string;
    connGen: number;
  } | null>(null);

  useEffect(() => {
    if (!pending) return;
    if (
      pending.sessionId !== sessionId ||
      pending.task.key !== task.key ||
      currentGrant !== pending.grant
    ) {
      setPending(null);
    }
  }, [pending, sessionId, task.key, currentGrant]);

  const open = Boolean(
    pending &&
      pending.sessionId === sessionId &&
      pending.task.key === task.key &&
      currentGrant === pending.grant,
  );
  const stopMessage = stop && stop.outcome !== "stopping"
    ? (stop.message ? `${STOP_MESSAGE[stop.outcome]}: ${stop.message}` : STOP_MESSAGE[stop.outcome])
    : null;

  const trigger = (): void => {
    if (currentGrant) {
      setPending({ sessionId, task, grant: currentGrant, connGen: connectionGeneration() });
    }
  };
  const close = (): void => setPending(null);
  const confirm = (): void => {
    const captured = pending;
    setPending(null);
    if (!captured) return;
    if (captured.sessionId !== sessionId || captured.task.key !== task.key) return;
    const liveConnGen = connectionGeneration();
    if (liveConnGen !== captured.connGen) return;
    if (
      stopGrantKey(sessionId, task, connected, disconnectEpochRef.current, liveConnGen) !==
        captured.grant
    ) return;
    if (!matchesLiveCanonicalTask(captured.sessionId, captured.task)) return;
    requestTaskStop(captured.sessionId, captured.task, options?.stopSender);
  };

  return {
    open,
    pending,
    stopStopping: stop?.outcome === "stopping",
    stopMessage,
    trigger,
    close,
    confirm,
  };
}
