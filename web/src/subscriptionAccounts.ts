import { useEffect, useRef, useSyncExternalStore } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import { type SubscriptionDisplay, validSubscriptionDisplay } from "./subscriptionDisplay";

export interface SubscriptionAccount {
  id: string;
  provider_id: string;
  label: string;
  enabled: boolean;
  revoking?: boolean;
}
export interface SubscriptionSnapshot {
  dataset: { dataset_id: string; user_id: string };
  registry: { revision: number; accounts: SubscriptionAccount[]; defaults: Record<string, string> };
  display_preferences?: SubscriptionDisplay;
  session_admission: string;
  session_providers?: string[];
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function text(value: unknown): value is string { return typeof value === "string" && value.length > 0; }

export function validSubscriptionSnapshot(value: unknown, owner: string | undefined): value is SubscriptionSnapshot {
  if (!owner || !record(value) || !record(value.dataset) || !record(value.registry)) return false;
  const { dataset, registry } = value;
  return dataset.user_id === owner && text(dataset.dataset_id) &&
    typeof registry.revision === "number" && Number.isSafeInteger(registry.revision) && registry.revision >= 0 &&
    (value.display_preferences === undefined || validSubscriptionDisplay(value.display_preferences)) &&
    (value.session_providers === undefined || (Array.isArray(value.session_providers) && value.session_providers.every(text))) &&
    typeof value.session_admission === "string" && record(registry.defaults) && Object.values(registry.defaults).every(text) &&
    Array.isArray(registry.accounts) && registry.accounts.every((account: unknown) => record(account) &&
      text(account.id) && text(account.provider_id) && text(account.label) && typeof account.enabled === "boolean" &&
      (account.revoking === undefined || typeof account.revoking === "boolean"));
}

const empty: { data?: SubscriptionSnapshot; error: string } = { error: "" };
let snapshot = empty;
let snapshotOwner: string | undefined;
let pending: Promise<void> | undefined;
let invalidated = false;
const listeners = new Set<() => void>();
function current() {
  return !productSessionSignal().aborted && snapshotOwner === productSyncPrincipal() ? snapshot : empty;
}
function publish() { for (const listener of listeners) listener(); }

/** A single inventory request for the whole UI, including many session/call badges.
 * A preference event during a request schedules another read instead of losing it. */
async function refresh(): Promise<void> {
  if (productSessionSignal().aborted || !productSyncPrincipal()) return;
  if (pending) return pending;
  pending = (async () => {
    do {
      invalidated = false;
      const owner = productSyncPrincipal();
      const signal = productSessionSignal();
      try {
        const response = await fetch("/api/provider-accounts", { cache: "no-store", signal });
        if (!response.ok) throw new Error("Could not load registered accounts.");
        const value: unknown = await response.json();
        if (signal.aborted || owner !== productSyncPrincipal()) return;
        if (!validSubscriptionSnapshot(value, owner)) throw new Error("Account inventory changed. Reload to continue.");
        const old = current().data;
        if (!old || old.dataset.dataset_id !== value.dataset.dataset_id ||
          (value.registry.revision >= old.registry.revision && (value.display_preferences?.revision ?? 0) >= (old.display_preferences?.revision ?? 0))
        ) { snapshot = { data: value, error: "" }; snapshotOwner = owner; }
      } catch (cause) {
        if (!signal.aborted && owner === productSyncPrincipal()) {
          snapshot = { ...current(), error: cause instanceof Error ? cause.message : "Could not load registered accounts." };
          snapshotOwner = owner;
        }
      }
      publish();
    } while (invalidated && !productSessionSignal().aborted);
  })().finally(() => { pending = undefined; });
  return pending;
}
function changed(event: Event) {
  const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
  if (!scopes || Object.hasOwn(scopes, "provider_accounts") || Object.hasOwn(scopes, "subscriptions")) {
    invalidated = true;
    void refresh();
  }
}
function subscribe(listener: () => void) {
  if (listeners.size === 0) {
    window.addEventListener(PREFERENCES_CHANGED, changed);
    productSessionSignal().addEventListener("abort", clear, { once: true });
  }
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      invalidated = true;
      window.removeEventListener(PREFERENCES_CHANGED, changed);
      productSessionSignal().removeEventListener("abort", clear);
    }
  };
}
function clear() { snapshot = empty; snapshotOwner = undefined; publish(); }

/** Read-only account inventory; mutations continue through the shared writer. */
export function useSubscriptionAccounts(connected = true) {
  const value = useSyncExternalStore(subscribe, current, () => empty);
  const previous = useRef(connected);
  useEffect(() => {
    if (!current().data || invalidated || (connected && !previous.current)) void refresh();
    previous.current = connected;
  }, [connected]);
  return { ...value, refresh };
}
