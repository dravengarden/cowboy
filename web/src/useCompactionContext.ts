import { useCallback, useEffect, useState } from "react";
import type { Status } from "./protocol";

interface ContextValue {
  used: number;
  size: number;
}

interface ContextOverride extends ContextValue {
  serverUsed: number;
  serverSize: number;
}

interface CompactRequest {
  baseline: ContextValue;
  sawBusy: boolean;
  startedAt: number;
}

/**
 * Bridge the short gap between an agent finishing `/compact` and the canonical
 * sessions broadcast reaching the client. The per-session info endpoint reads
 * the same live Hub metadata, so it is a safe read-through while WebSocket
 * delivery catches up. The broadcast always wins as soon as it changes.
 */
export function useCompactionContext({
  sessionId,
  status,
  serverUsed,
  serverSize,
}: {
  sessionId: string;
  status: Status;
  serverUsed: number;
  serverSize: number;
}): {
  used: number;
  size: number;
  refreshing: boolean;
  beginRefresh: () => void;
} {
  const [request, setRequest] = useState<CompactRequest | null>(null);
  const [override, setOverride] = useState<ContextOverride | null>(null);

  const beginRefresh = useCallback((): void => {
    setOverride(null);
    setRequest({
      baseline: { used: serverUsed, size: serverSize },
      sawBusy: false,
      startedAt: Date.now(),
    });
  }, [serverSize, serverUsed]);

  useEffect(() => {
    if (request && status === "busy" && !request.sawBusy) {
      setRequest({ ...request, sawBusy: true });
    }
  }, [request, status]);

  useEffect(() => {
    if (
      override &&
      (serverUsed !== override.serverUsed || serverSize !== override.serverSize)
    ) {
      setOverride(null);
    }
  }, [override, serverSize, serverUsed]);

  useEffect(() => {
    if (!request) return;
    const { baseline } = request;

    // A live sessions broadcast is canonical. During the turn it can contain
    // intermediate usage, so only settle after the compact turn is no longer
    // busy. This is the race that previously left the ring at its old value.
    if (
      status !== "busy" &&
      (serverUsed !== baseline.used || serverSize !== baseline.size)
    ) {
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
        const response = await fetch(
          `/api/sessions/${encodeURIComponent(sessionId)}/info`,
          { cache: "no-store" },
        );
        if (response.ok) {
          const info = await response.json() as {
            meta?: { context_used?: number; context_size?: number };
          };
          const used = info.meta?.context_used ?? 0;
          const size = info.meta?.context_size ?? 0;
          if (used !== baseline.used || size !== baseline.size) {
            if (!cancelled) {
              setOverride({ used, size, serverUsed, serverSize });
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
  }, [request, serverSize, serverUsed, sessionId, status]);

  return {
    used: override?.used ?? serverUsed,
    size: override?.size ?? serverSize,
    refreshing: request !== null,
    beginRefresh,
  };
}
