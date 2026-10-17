import { useCallback, useEffect, useRef, useState } from "react";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";

export type OrderKey = "usage_order" | "machine_order";
export interface AboutValues { usage_order: string[]; machine_order: string[] }
export interface PreferenceSnapshot { scope: "about"; revision: number; values: AboutValues }
export interface OrderChoice { id: string; name: string }
export interface AboutPreferences {
  preferences: PreferenceSnapshot;
  choices: Record<OrderKey, OrderChoice[]>;
  dataset: { dataset_id: string; user_id: string };
}
export const PREFERENCES_CHANGED = "cowboy:preferences-changed";
export const DEFAULT_ABOUT_VALUES: AboutValues = { usage_order: [], machine_order: [] };

/** Unknown IDs stay dormant; new items append in their original default order. */
export function orderedItems<T>(items: readonly T[], order: readonly string[], id: (item: T) => string): T[] {
  const rank = new Map(order.map((key, index) => [key, index]));
  return [...items].sort((a, b) => (rank.get(id(a)) ?? Infinity) - (rank.get(id(b)) ?? Infinity));
}

export function useAboutPreferences(connected: boolean) {
  const [data, setData] = useState<AboutPreferences>();
  const [error, setError] = useState<string>();
  const [loadError, setLoadError] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [pending, setPending] = useState<{ operation: string; scope: "about"; expected_revision: number; changes: Partial<AboutValues> }>();
  const alive = useRef(false);
  const dataRef = useRef(data);
  dataRef.current = data;
  const loading = useRef(0);
  const writing = useRef(false);
  const load = useCallback(async () => {
    const sequence = ++loading.current;
    try {
      const response = await fetch("/api/preferences/about", { cache: "no-store", signal: productSessionSignal() });
      if (!response.ok) throw new Error("Could not load layout preferences. Retry when connected.");
      const next = await response.json() as AboutPreferences;
      if (!alive.current || sequence !== loading.current || productSessionSignal().aborted) return;
      if (next.dataset.user_id !== productSyncPrincipal()) throw new Error("Account changed. Reload before editing the layout.");
      setData((old) => !old || next.preferences.revision >= old.preferences.revision ? next : old);
      setLoadError(undefined);
    } catch (cause) { if (alive.current) setLoadError(String(cause)); }
  }, []);
  useEffect(() => {
    alive.current = true;
    void load();
    const refresh = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (scopes && (!scopes.about || scopes.about <= (dataRef.current?.preferences.revision ?? -1))) return;
      void load();
    };
    window.addEventListener(PREFERENCES_CHANGED, refresh);
    return () => { alive.current = false; ++loading.current; window.removeEventListener(PREFERENCES_CHANGED, refresh); };
  }, [load, connected]);

  const commit = async (input: NonNullable<typeof pending>): Promise<void> => {
    const current = dataRef.current;
    if (!current || writing.current || productSessionSignal().aborted) return;
    writing.current = true;
    setSaving(true);
    setPending(input);
    setError(undefined);
    try {
      const response = await fetch("/api/preferences", {
        method: "PUT", signal: productSessionSignal(),
        headers: { "Content-Type": "application/json", "x-cowboy-dataset": current.dataset.dataset_id },
        body: JSON.stringify(input),
      });
      const result = await response.json() as { preferences?: PreferenceSnapshot; current?: PreferenceSnapshot; error?: { code: string } };
      if (!alive.current || productSessionSignal().aborted) return;
      if (response.status === 409 && result.current) {
        setData((old) => old && result.current!.revision >= old.preferences.revision ? { ...old, preferences: result.current! } : old);
        setError("Layout changed elsewhere. Your proposed order is kept below. Apply it again to replace the latest order, or discard it.");
        // A rejected CAS has no receipt. Only an explicit retry may use a fresh revision.
        setPending({ ...input, operation: crypto.randomUUID(), expected_revision: result.current.revision });
        return;
      }
      if (!response.ok || !result.preferences) throw new Error(result.error?.code ?? "Could not save layout");
      setData((old) => old && result.preferences!.revision >= old.preferences.revision ? { ...old, preferences: result.preferences! } : old);
      setPending(undefined);
    } catch (cause) {
      if (alive.current) setError(`${String(cause)}. Your order is kept; retry uses the same operation.`);
    } finally { writing.current = false; if (alive.current) setSaving(false); }
  };
  return {
    data, error: error ?? loadError, saving, pending,
    save: (changes: Partial<AboutValues>) => data && !pending
      ? commit({ operation: crypto.randomUUID(), scope: "about", expected_revision: data.preferences.revision, changes })
      : Promise.resolve(),
    retry: () => pending ? commit(pending) : load(),
    discard: () => { setPending(undefined); setError(undefined); void load(); },
  };
}
