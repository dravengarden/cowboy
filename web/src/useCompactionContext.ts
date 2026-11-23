import { useCallback, useEffect, useState } from "react";
import type { Status } from "./protocol";
import {
  contextValueFromSessionInfo,
  type ContextReading,
  freshContextReading,
} from "./sessionInfoContext";

/** A read-through of the Controller's reading, kept until the broadcast moves. */
interface ContextOverride extends ContextReading {
  server: ContextReading;
}

interface CompactRequest {
  baseline: ContextReading;
  completionSeq: number;
  sawBusy: boolean;
  startedAt: number;
}

function sameReading(a: ContextReading, b: ContextReading): boolean {
  return (
    a.used === b.used && a.size === b.size && a.observedAt === b.observedAt && a.stale === b.stale
  );
}

/**
 * The Controller marks a reading that a completed compaction left without a
 * fresh report (`serverStale`): the meter then shows unknown, on every device
 * and after a reload, until the agent's next usage report replaces it.
 *
 * Bridge the short gap between an agent finishing `/compact` and the canonical
 * sessions broadcast reaching the client. The per-session info endpoint reads
 * the same live Hub metadata, so it is a safe read-through while WebSocket
 * delivery catches up. Both inputs settle the refresh only with a reading the
 * Controller marked stale or one measured after the click (its observation
 * time, not its counters, identifies it). The broadcast always wins as soon as
 * it changes.
 */
export function useCompactionContext({
  sessionId,
  status,
  serverUsed,
  serverSize,
  serverObservedAt = 0,
  serverStale = false,
  completionSeq,
}: {
  sessionId: string;
  status: Status;
  serverUsed: number;
  serverSize: number;
  /** `usage.observed_at_ms` of the broadcast reading; 0 when unknown. */
  serverObservedAt?: number;
  serverStale?: boolean;
  completionSeq: number;
}): {
  used: number;
  size: number;
  /** The reading is history after a compaction; show it as unknown. */
  stale: boolean;
  refreshing: boolean;
  beginRefresh: () => void;
  /** A refused command never starts, so nothing would end the refresh. */
  cancelRefresh: () => void;
} {
  const [request, setRequest] = useState<CompactRequest | null>(null);
  const [override, setOverride] = useState<ContextOverride | null>(null);

  const cancelRefresh = useCallback((): void => setRequest(null), []);

  const beginRefresh = useCallback((): void => {
    setOverride(null);
    setRequest({
      baseline: {
        used: serverUsed,
        size: serverSize,
        observedAt: serverObservedAt,
        stale: serverStale,
      },
      completionSeq,
      sawBusy: false,
      startedAt: Date.now(),
    });
  }, [completionSeq, serverObservedAt, serverSize, serverStale, serverUsed]);

  // The canonical mark settles a refresh either way: set, nothing newer
  // exists to poll for; cleared, the server holds the fresh reading, even
  // one that repeats the old counters.
  useEffect(() => {
    setRequest(null);
    setOverride(null);
  }, [serverStale]);

  useEffect(() => {
    if (request && status === "busy" && !request.sawBusy) {
      setRequest({ ...request, sawBusy: true });
    }
  }, [request, status]);

  useEffect(() => {
    if (
      override &&
      !sameReading(override.server, {
        used: serverUsed,
        size: serverSize,
        observedAt: serverObservedAt,
        stale: serverStale,
      })
    ) {
      setOverride(null);
    }
  }, [override, serverObservedAt, serverSize, serverStale, serverUsed]);

  useEffect(() => {
    if (!request) return;
    const server: ContextReading = {
      used: serverUsed,
      size: serverSize,
      observedAt: serverObservedAt,
      stale: serverStale,
    };
    const { baseline } = request;

    // A live sessions broadcast is canonical. During the turn it can contain
    // intermediate usage, so only settle after the compact turn has completed
    // or is no longer busy. This is the race that previously left the ring at
    // its old value.
    const ended = status !== "busy" || completionSeq > request.completionSeq;
    if (ended && freshContextReading(server, baseline)) {
      setOverride(null);
      setRequest(null);
      return;
    }
    if (status === "busy") return;

    let cancelled = false;
    let attempts = 0;
    const refresh = async (): Promise<void> => {
      attempts += 1;
      try {
        const response = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}/info`, {
          cache: "no-store",
        });
        if (response.ok) {
          const value = contextValueFromSessionInfo(await response.json());
          if (!value) throw new Error("invalid session info context usage");
          if ((value.stale && !baseline.stale) || freshContextReading(value, baseline)) {
            if (!cancelled) {
              setOverride({ ...value, server });
              setRequest(null);
            }
            return;
          }
        }
      } catch {
        // A transient HTTP miss is harmless: the sessions broadcast remains
        // authoritative and the bounded retry below keeps the UI responsive.
      }
      if (!cancelled && attempts < 40) {
        window.setTimeout(() => void refresh(), 500);
      } else if (!cancelled) {
        setRequest(null);
      }
    };

    // Normally wait until the observed busy turn ends. Very fast providers can
    // complete between React renders, so retain a small fallback delay before
    // polling instead of requiring `sawBusy` to have been rendered.
    const elapsed = Date.now() - request.startedAt;
    const delay = request.sawBusy ? 100 : Math.max(100, 750 - elapsed);
    const timer = window.setTimeout(() => void refresh(), delay);
    return (): void => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [
    completionSeq,
    request,
    serverObservedAt,
    serverSize,
    serverStale,
    serverUsed,
    sessionId,
    status,
  ]);

  if (serverStale) {
    return {
      used: serverUsed,
      size: serverSize,
      stale: true,
      refreshing: false,
      beginRefresh,
      cancelRefresh,
    };
  }
  return {
    used: override?.used ?? serverUsed,
    size: override?.size ?? serverSize,
    stale: override?.stale ?? false,
    refreshing: request !== null,
    beginRefresh,
    cancelRefresh,
  };
}
