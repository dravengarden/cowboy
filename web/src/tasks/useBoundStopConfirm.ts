// Shared Stop confirmation guard binding an open confirmation dialog to the
// exact session, task identity, latest observed execution snapshot, and live
// connection epoch so a resumed subagent run or newer worker can never be
// stopped by a stale dialog.

import { useEffect, useRef, useState } from "react";
import type { TaskSummary } from "../backgroundTasks";
import { useStoreSelector } from "../store";
import { requestTaskStop, STOP_MESSAGE, type StopState, useStopStates } from "../useBackgroundTasks";

export function stopGrantKey(
  sessionId: string,
  task: TaskSummary,
  connected: boolean,
  disconnectEpoch: number,
): string | null {
  if (!task.canStop) return null;
  const connPart = `${connected ? "1" : "0"}:${String(disconnectEpoch)}`;
  if (task.source === "subagent" && task.subagent) {
    const sub = task.subagent;
    return `${sessionId}\u0000subagent:${sub.id}\u0000${sub.incarnation ?? ""}\u0000rev:${String(sub.revision)}\u0000seq:${String(sub.observedSeq ?? "")}\u0000term:${String(sub.terminalSeq ?? "")}\u0000${connPart}`;
  }
  if (task.source === "call" && task.call) {
    return `${sessionId}\u0000call:${task.call.call_id}\u0000${task.call.state}\u0000${String(task.call.updated_at_ms)}\u0000${connPart}`;
  }
  return null;
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
  const prevConnectedRef = useRef(connected);
  const disconnectEpochRef = useRef(0);
  if (prevConnectedRef.current && !connected) {
    disconnectEpochRef.current += 1;
  }
  prevConnectedRef.current = connected;

  const currentGrant = stopGrantKey(sessionId, task, connected, disconnectEpochRef.current);
  const [pending, setPending] = useState<{ sessionId: string; task: TaskSummary; grant: string } | null>(null);

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
    if (currentGrant) setPending({ sessionId, task, grant: currentGrant });
  };
  const close = (): void => setPending(null);
  const confirm = (): void => {
    const captured = pending;
    setPending(null);
    if (!captured) return;
    if (captured.sessionId !== sessionId || captured.task.key !== task.key) return;
    if (stopGrantKey(sessionId, task, connected, disconnectEpochRef.current) !== captured.grant) return;
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
