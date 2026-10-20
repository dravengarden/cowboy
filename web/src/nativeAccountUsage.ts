import { useEffect, useSyncExternalStore } from "react";
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

const empty: { accounts: NativeAccountUsage[]; error: string } = { accounts: [], error: "" };
let snapshot = empty;
let snapshotOwner: string | undefined;
let pending: Promise<void> | undefined;
let invalidated = false;
let lastRevision: number | undefined;
let stopPolling: (() => void) | undefined;
const listeners = new Set<() => void>();
function current() {
  return snapshotOwner === productSyncPrincipal() && !productSessionSignal().aborted ? snapshot : empty;
}
function publish() { for (const listener of listeners) listener(); }

/** Every mounted surface shares this owner-scoped read and refresh state. */
async function load(): Promise<void> {
  if (!productSyncPrincipal() || productSessionSignal().aborted) return;
  if (pending) return pending;
  pending = (async () => {
    do {
      invalidated = false;
      const owner = productSyncPrincipal();
      const signal = productSessionSignal();
      try {
        const response = await fetch("/api/provider-accounts/usage", { signal });
        await expectHttpOk(response, "Could not load account usage");
        const value = await response.json() as { accounts: NativeAccountUsage[] };
        if (!Array.isArray(value.accounts)) throw new Error("Account usage is not available from this Service.");
        if (signal.aborted || owner !== productSyncPrincipal()) return;
        snapshot = { accounts: value.accounts, error: "" };
        snapshotOwner = owner;
      } catch (cause) {
        if (signal.aborted || owner !== productSyncPrincipal()) return;
        snapshot = { ...current(), error: cause instanceof Error ? cause.message : "Could not load account usage" };
        snapshotOwner = owner;
      }
      publish();
    } while (invalidated && !productSessionSignal().aborted);
  })().finally(() => {
    pending = undefined;
    if (invalidated && listeners.size > 0 && !productSessionSignal().aborted) return load();
    return undefined;
  });
  return pending;
}

function reload() { invalidated = true; return load(); }
function changed(event: Event) {
  if ((event as CustomEvent<string>).detail === productSyncPrincipal()) void reload();
}
function focus() { if (document.visibilityState !== "hidden") void reload(); }
function clear() {
  snapshot = empty;
  snapshotOwner = undefined;
  lastRevision = undefined;
  publish();
}
function subscribe(listener: () => void) {
  if (listeners.size === 0) {
    window.addEventListener("cowboy:account-usage-changed", changed);
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", focus);
    productSessionSignal().addEventListener("abort", clear, { once: true });
    stopPolling = lowPriorityInterval(() => void reload(), 30_000);
  }
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      stopPolling?.();
      window.removeEventListener("cowboy:account-usage-changed", changed);
      window.removeEventListener("focus", focus);
      document.removeEventListener("visibilitychange", focus);
      productSessionSignal().removeEventListener("abort", clear);
      lastRevision = undefined;
    }
  };
}
async function refresh(id: string): Promise<void> {
  const signal = productSessionSignal();
  const response = await fetch(`/api/provider-accounts/${encodeURIComponent(id)}/usage`, { method: "POST", signal });
  await expectHttpOk(response, "Could not refresh account usage");
  if (!signal.aborted) await reload();
}

export function useNativeAccountUsage(revision?: number) {
  const value = useSyncExternalStore(subscribe, current, () => empty);
  useEffect(() => {
    if (revision !== undefined && (revision !== lastRevision || snapshotOwner !== productSyncPrincipal())) {
      lastRevision = revision;
      void reload();
    }
  }, [revision]);
  return { ...value, load: reload, refresh };
}
