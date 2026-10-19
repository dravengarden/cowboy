import { useEffect, useState } from "react";
import type { DeliveryTarget } from "./currentTurnDelivery";

export type DeliveryMode = "steer" | "tool_boundary" | "next_turn";
export interface DeliveryPolicy {
  mode: DeliveryMode;
  revision: number;
}
export interface DeliveryOptions {
  target: DeliveryTarget | null;
  steer: boolean;
  controls: { version: number; next_checkpoint: boolean };
  status: string;
  paused: boolean;
  hold: string | null;
  policy: DeliveryPolicy;
}

export async function readDeliveryOptions(
  session: string,
  signal?: AbortSignal,
): Promise<DeliveryOptions> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(session)}/delivery-options`,
    {
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(5000)])
        : AbortSignal.timeout(5000),
      cache: "no-store",
    },
  );
  if (!response.ok) throw new Error("Delivery options are unavailable.");
  return await response.json() as DeliveryOptions;
}

const changed = "cowboy:delivery-policy";

export async function saveDeliveryPolicy(
  session: string,
  policy: DeliveryPolicy,
): Promise<void> {
  let response: Response;
  try {
    response = await fetch(
      `/api/sessions/${encodeURIComponent(session)}/delivery-policy`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(policy),
        signal: AbortSignal.timeout(5000),
      },
    );
    // A conflict or lost acknowledgement requires a fresh authoritative read.
  } finally {
    globalThis.dispatchEvent(new Event(changed));
  }
  if (response.status === 409) {
    throw new Error(
      "Queue policy changed elsewhere. Review the current value.",
    );
  }
  if (!response.ok) throw new Error("Could not save queue policy.");
}

export async function releaseDeliveryHold(
  session: string,
  operation: string,
): Promise<void> {
  try {
    const response = await fetch(
      `/api/sessions/${encodeURIComponent(session)}/delivery-hold/release`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ operation }),
        signal: AbortSignal.timeout(5000),
      },
    );
    if (!response.ok) {
      throw new Error(
        "Wait for the worker to become idle before releasing this hold.",
      );
    }
  } finally {
    globalThis.dispatchEvent(new Event(changed));
  }
}

export function useDeliveryOptions(session: string, enabled = true): {
  options: DeliveryOptions | null;
  error: string | null;
} {
  const [value, setValue] = useState<
    { session: string; options: DeliveryOptions | null; error: string | null }
  >({
    session,
    options: null,
    error: null,
  });
  useEffect(() => {
    if (!enabled) return;
    let controller: AbortController | undefined;
    const refresh = (): void => {
      controller?.abort();
      controller = new AbortController();
      const signal = controller.signal;
      void readDeliveryOptions(session, signal).then((options) => {
        if (!signal.aborted) setValue({ session, options, error: null });
      }).catch((error: unknown) => {
        if (!signal.aborted) {
          setValue({ session, options: null, error: String(error) });
        }
      });
    };
    refresh();
    globalThis.addEventListener(changed, refresh);
    globalThis.addEventListener("focus", refresh);
    return () => {
      controller?.abort();
      globalThis.removeEventListener(changed, refresh);
      globalThis.removeEventListener("focus", refresh);
    };
  }, [session, enabled]);
  return value.session === session ? value : { options: null, error: null };
}
