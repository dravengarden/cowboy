import { useCallback, useEffect, useRef, useState } from "react";
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

/** Read-only account inventory; mutations continue through the shared writer. */
export function useSubscriptionAccounts(connected: boolean) {
  const [data, setData] = useState<SubscriptionSnapshot>();
  const [error, setError] = useState("");
  const alive = useRef(false);
  const sequence = useRef(0);
  const refresh = useCallback(async (): Promise<void> => {
    const request = ++sequence.current;
    const owner = productSyncPrincipal();
    const signal = productSessionSignal();
    try {
      const response = await fetch("/api/provider-accounts", { cache: "no-store", signal });
      if (!response.ok) throw new Error("Could not load registered accounts.");
      const value: unknown = await response.json();
      if (!alive.current || request !== sequence.current || signal.aborted || owner !== productSyncPrincipal()) return;
      if (!validSubscriptionSnapshot(value, owner)) throw new Error("Account inventory changed. Reload to continue.");
      setData((old) => !old || old.dataset.dataset_id !== value.dataset.dataset_id ||
        (value.registry.revision >= old.registry.revision && (value.display_preferences?.revision ?? 0) >= (old.display_preferences?.revision ?? 0))
        ? value : old);
      setError("");
    } catch (cause) {
      if (alive.current && request === sequence.current && !signal.aborted) {
        setError(cause instanceof Error ? cause.message : "Could not load registered accounts.");
      }
    }
  }, []);
  useEffect(() => {
    alive.current = true;
    void refresh();
    const changed = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (!scopes || Object.hasOwn(scopes, "provider_accounts") || Object.hasOwn(scopes, "subscriptions")) void refresh();
    };
    window.addEventListener(PREFERENCES_CHANGED, changed);
    return () => {
      alive.current = false;
      ++sequence.current;
      window.removeEventListener(PREFERENCES_CHANGED, changed);
    };
  }, [connected, refresh]);
  return { data: data?.dataset.user_id === productSyncPrincipal() ? data : undefined, error, refresh };
}
