import { useCallback, useEffect, useRef, useState } from "react";
import { expectHttpOk } from "./httpResponse";
import { productSyncPrincipal } from "./productSyncIdentity";
import { productSessionSignal } from "./productSessionEnd";
import { lowPriorityInterval } from "./network/bandwidth";
import type { ProviderUsage } from "./usageLimits";

export interface NativeAccountUsage {
  account_id: string;
  provider_id: string;
  machine_id?: string;
  available?: boolean;
  usage?: ProviderUsage | null;
  refreshing: boolean;
  reason?: string;
  message?: string;
  next_refresh_at_ms?: number;
}

/** Cached observations are owner-scoped and discarded across product sign-in. */
export function useNativeAccountUsage(revision: number | undefined) {
  const owner = productSyncPrincipal();
  const [snapshot, setSnapshot] = useState<{ owner: string | undefined; accounts: NativeAccountUsage[] }>();
  const [error, setError] = useState("");
  const sequence = useRef(0);
  const load = useCallback(async (signal?: AbortSignal) => {
    const request = ++sequence.current;
    try {
      const response = await fetch("/api/provider-accounts/usage", signal ? { signal } : {});
      await expectHttpOk(response, "Could not load account usage");
      const value = await response.json() as { accounts: NativeAccountUsage[] };
      if (!Array.isArray(value.accounts)) throw new Error("Account usage is not available from this Service.");
      if (request === sequence.current && owner === productSyncPrincipal() && !signal?.aborted && !productSessionSignal().aborted) {
        setSnapshot({ owner, accounts: value.accounts });
        setError("");
      }
    } catch (cause) {
      if (request === sequence.current && !signal?.aborted && !productSessionSignal().aborted && owner === productSyncPrincipal()) setError(cause instanceof Error ? cause.message : "Could not load account usage");
    }
  }, [owner]);
  const refresh = useCallback(async (id: string) => {
    const response = await fetch(`/api/provider-accounts/${encodeURIComponent(id)}/usage`, { method: "POST" });
    await expectHttpOk(response, "Could not refresh account usage");
    await load();
  }, [load]);
  useEffect(() => {
    if (revision === undefined) return;
    const controller = new AbortController();
    void load(controller.signal);
    const stop = lowPriorityInterval(() => void load(controller.signal), 30_000);
    const focus = () => { if (document.visibilityState !== "hidden") void load(controller.signal); };
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", focus);
    return () => {
      controller.abort(); stop();
      window.removeEventListener("focus", focus);
      document.removeEventListener("visibilitychange", focus);
    };
  }, [revision, load]);
  return { accounts: snapshot && snapshot.owner === owner ? snapshot.accounts : [], error, load, refresh };
}
