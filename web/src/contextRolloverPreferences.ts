import { useCallback, useEffect, useRef, useState } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";

export interface ContextRolloverValues {
  enabled: boolean;
  min_context_tokens: number;
  idle_minutes: Record<string, number | null>;
}

export interface ContextRolloverProviderChoice {
  id: string;
  name: string;
  builtin_idle_minutes: number | null;
  recommended_idle_minutes: number | null;
  policy_restricted: boolean;
  restriction: string | null;
  keepalive_independent: boolean;
}

export interface ContextRolloverSnapshot {
  scope: "context_rollover";
  revision: number;
  values: ContextRolloverValues;
}

export interface ContextRolloverPreferencesPayload {
  preferences: ContextRolloverSnapshot;
  providers: ContextRolloverProviderChoice[];
  dataset: { dataset_id: string; user_id: string };
}

export const DEFAULT_MIN_CONTEXT_TOKENS = 80_000;
export const DEFAULT_CONTEXT_ROLLOVER_VALUES: ContextRolloverValues = {
  enabled: true,
  min_context_tokens: DEFAULT_MIN_CONTEXT_TOKENS,
  idle_minutes: {},
};

export const MIN_CONTEXT_TOKEN_PRESETS: readonly number[] = [
  40_000,
  60_000,
  80_000,
  120_000,
  200_000,
];

const STANDARD_IDLE_PRESETS: readonly number[] = [30, 60, 120, 240, 360, 720];

export function withContextRolloverDefaults(
  snapshot: ContextRolloverSnapshot,
): ContextRolloverSnapshot {
  const raw = snapshot.values as Partial<ContextRolloverValues> | undefined;
  return {
    ...snapshot,
    values: {
      enabled: typeof raw?.enabled === "boolean" ? raw.enabled : true,
      min_context_tokens:
        typeof raw?.min_context_tokens === "number" && Number.isFinite(raw.min_context_tokens)
          ? raw.min_context_tokens
          : DEFAULT_MIN_CONTEXT_TOKENS,
      idle_minutes:
        raw?.idle_minutes && typeof raw.idle_minutes === "object"
          ? { ...raw.idle_minutes }
          : {},
    },
  };
}

export function formatIdleMinutesLabel(minutes: number): string {
  if (minutes % 60 === 0) {
    const hours = minutes / 60;
    return `${hours} h`;
  }
  return `${minutes} min`;
}

/** Resolve the effective idle threshold for a Provider, always honoring host policy. */
export function effectiveProviderIdleMinutes(
  values: ContextRolloverValues,
  provider: ContextRolloverProviderChoice,
): number | null {
  if (provider.policy_restricted) return null;
  if (Object.prototype.hasOwnProperty.call(values.idle_minutes, provider.id)) {
    const configured = values.idle_minutes[provider.id];
    return typeof configured === "number" && configured >= 1 ? configured : null;
  }
  return provider.builtin_idle_minutes;
}

/** Return the updated `idle_minutes` map after changing one Provider's threshold.
 *  Elides entries that match the built-in default so preferences stay minimal,
 *  and never writes a non-null threshold for a host-policy-restricted Provider. */
export function withProviderIdleMinutes(
  values: ContextRolloverValues,
  provider: ContextRolloverProviderChoice,
  nextMinutes: number | null,
): Record<string, number | null> {
  const next: Record<string, number | null> = { ...values.idle_minutes };
  if (provider.policy_restricted) {
    delete next[provider.id];
    return next;
  }
  if (nextMinutes === provider.builtin_idle_minutes) {
    delete next[provider.id];
  } else {
    next[provider.id] = nextMinutes;
  }
  return next;
}

export interface IdleChoiceOption {
  value: string;
  label: string;
}

export function providerIdleChoices(
  provider: ContextRolloverProviderChoice,
  currentMinutes: number | null,
): IdleChoiceOption[] {
  const minutesSet = new Set<number>(STANDARD_IDLE_PRESETS);
  if (provider.builtin_idle_minutes !== null) minutesSet.add(provider.builtin_idle_minutes);
  if (provider.recommended_idle_minutes !== null) minutesSet.add(provider.recommended_idle_minutes);
  if (currentMinutes !== null && currentMinutes >= 1) minutesSet.add(currentMinutes);
  const sorted = [...minutesSet].sort((a, b) => a - b);

  const offLabel = provider.builtin_idle_minutes === null ? "Off · default" : "Off";
  const options: IdleChoiceOption[] = [{ value: "off", label: offLabel }];
  for (const minutes of sorted) {
    const base = formatIdleMinutesLabel(minutes);
    let suffix = "";
    if (minutes === provider.builtin_idle_minutes) {
      suffix = " · default";
    } else if (minutes === provider.recommended_idle_minutes) {
      suffix = provider.id === "codex" ? " · canary" : " · recommended";
    }
    options.push({ value: String(minutes), label: `${base}${suffix}` });
  }
  return options;
}

export function providerRolloverDescription(
  provider: ContextRolloverProviderChoice,
): string {
  if (provider.policy_restricted) {
    return provider.restriction ?? "Automatic context renewal is disabled by host policy.";
  }
  const parts: string[] = [];
  if (provider.builtin_idle_minutes !== null) {
    parts.push(`Built-in default: ${formatIdleMinutesLabel(provider.builtin_idle_minutes)}`);
  } else if (provider.recommended_idle_minutes !== null) {
    const tag = provider.id === "codex" ? "canary" : "recommended";
    parts.push(
      `Off by default · ${formatIdleMinutesLabel(provider.recommended_idle_minutes)} ${tag}`,
    );
  } else {
    parts.push("Off by default · opt-in");
  }
  if (provider.keepalive_independent) {
    parts.push("Independent from DeepSeek cache protection");
  }
  return parts.join(" · ");
}

interface PendingRolloverWrite {
  operation: string;
  scope: "context_rollover";
  expected_revision: number;
  changes: Partial<ContextRolloverValues>;
}

export function useContextRolloverPreferences() {
  const [data, setData] = useState<ContextRolloverPreferencesPayload>();
  const [error, setError] = useState<string>();
  const [loadError, setLoadError] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [pending, setPending] = useState<PendingRolloverWrite>();
  const alive = useRef(false);
  const dataRef = useRef(data);
  dataRef.current = data;
  const loading = useRef(0);
  const writing = useRef(false);

  const load = useCallback(async () => {
    const sequence = ++loading.current;
    try {
      const response = await fetch("/api/preferences/context_rollover", {
        cache: "no-store",
        signal: productSessionSignal(),
      });
      if (!response.ok) {
        throw new Error("Could not load prompt-cache renewal preferences.");
      }
      const next = (await response.json()) as ContextRolloverPreferencesPayload;
      if (!alive.current || sequence !== loading.current || productSessionSignal().aborted) return;
      if (next.dataset.user_id !== productSyncPrincipal()) {
        throw new Error("Account changed. Reload before editing prompt-cache preferences.");
      }
      const normalized: ContextRolloverPreferencesPayload = {
        ...next,
        preferences: withContextRolloverDefaults(next.preferences),
        providers: Array.isArray(next.providers) ? next.providers : [],
      };
      setData((old) =>
        !old || next.preferences.revision >= old.preferences.revision ? normalized : old,
      );
      setLoadError(undefined);
    } catch (cause) {
      if (alive.current) setLoadError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void load();
    const refresh = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (
        scopes &&
        (!scopes.context_rollover ||
          scopes.context_rollover <= (dataRef.current?.preferences.revision ?? -1))
      ) {
        return;
      }
      void load();
    };
    window.addEventListener(PREFERENCES_CHANGED, refresh);
    return () => {
      alive.current = false;
      ++loading.current;
      window.removeEventListener(PREFERENCES_CHANGED, refresh);
    };
  }, [load]);

  const commit = async (input: PendingRolloverWrite): Promise<void> => {
    const current = dataRef.current;
    if (!current || writing.current || productSessionSignal().aborted) return;
    writing.current = true;
    setSaving(true);
    setPending(input);
    setError(undefined);
    try {
      const response = await fetch("/api/preferences", {
        method: "PUT",
        signal: productSessionSignal(),
        headers: {
          "Content-Type": "application/json",
          "x-cowboy-dataset": current.dataset.dataset_id,
        },
        body: JSON.stringify(input),
      });
      const result = (await response.json()) as {
        preferences?: ContextRolloverSnapshot;
        current?: ContextRolloverSnapshot;
        error?: { code: string };
      };
      if (!alive.current || productSessionSignal().aborted) return;
      if (response.status === 409 && result.current) {
        setData((old) =>
          old && result.current!.revision >= old.preferences.revision
            ? { ...old, preferences: withContextRolloverDefaults(result.current!) }
            : old,
        );
        setError("Prompt-cache settings changed elsewhere. Retry to apply your change or discard it.");
        setPending({
          ...input,
          operation: crypto.randomUUID(),
          expected_revision: result.current.revision,
        });
        return;
      }
      if (!response.ok || !result.preferences) {
        throw new Error(result.error?.code ?? "Could not save prompt-cache settings");
      }
      setData((old) =>
        old && result.preferences!.revision >= old.preferences.revision
          ? { ...old, preferences: withContextRolloverDefaults(result.preferences!) }
          : old,
      );
      setPending(undefined);
    } catch (cause) {
      if (alive.current) {
        const message = cause instanceof Error ? cause.message : String(cause);
        setError(`${message}. Retry uses the same operation.`);
      }
    } finally {
      writing.current = false;
      if (alive.current) setSaving(false);
    }
  };

  return {
    data,
    error: error ?? loadError,
    saving,
    pending,
    save: (changes: Partial<ContextRolloverValues>) =>
      data && !pending
        ? commit({
            operation: crypto.randomUUID(),
            scope: "context_rollover",
            expected_revision: data.preferences.revision,
            changes,
          })
        : Promise.resolve(),
    retry: () => (pending ? commit(pending) : load()),
    discard: () => {
      setPending(undefined);
      setError(undefined);
      void load();
    },
  };
}
