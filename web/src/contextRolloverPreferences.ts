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

export type ProviderIdleSelection = "default" | "off" | number;

export type RolloverIntent =
  | { kind: "enabled"; enabled: boolean }
  | { kind: "min_context_tokens"; min_context_tokens: number }
  | { kind: "provider_idle"; providerId: string; selection: ProviderIdleSelection }
  | { kind: "reset_defaults" };

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
    return `${minutes / 60} h`;
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

/** Return the Select control value distinguishing `default` (key absent) from
 *  explicit `off` (`null`) and numeric thresholds. */
export function providerIdleSelectionValue(
  values: ContextRolloverValues,
  provider: ContextRolloverProviderChoice,
): string {
  if (provider.policy_restricted) return "off";
  if (!Object.prototype.hasOwnProperty.call(values.idle_minutes, provider.id)) {
    return "default";
  }
  const configured = values.idle_minutes[provider.id];
  if (typeof configured === "number" && configured >= 1) {
    return String(configured);
  }
  return "off";
}

/** Drop obsolete non-null thresholds for host-policy-restricted Providers so
 *  legacy pre-policy rows never block edits to other Providers. */
export function sanitizeIdleMinutesMap(
  idleMinutes: Record<string, number | null>,
  providers: readonly ContextRolloverProviderChoice[],
): Record<string, number | null> {
  const restrictedIds = new Set(
    providers.filter((provider) => provider.policy_restricted).map((provider) => provider.id),
  );
  restrictedIds.add("antigravity");
  const next: Record<string, number | null> = {};
  for (const [providerId, minutes] of Object.entries(idleMinutes)) {
    if (restrictedIds.has(providerId) && minutes !== null) {
      continue;
    }
    next[providerId] = minutes;
  }
  return next;
}

/** Apply one Provider idle selection while preserving explicit `null` opt-outs
 *  (never erasing them to unconfigured defaults) and normalizing legacy
 *  restricted entries. */
export function withProviderIdleSelection(
  values: ContextRolloverValues,
  providers: readonly ContextRolloverProviderChoice[],
  provider: ContextRolloverProviderChoice,
  selection: ProviderIdleSelection,
): Record<string, number | null> {
  const next = sanitizeIdleMinutesMap(values.idle_minutes, providers);
  if (provider.policy_restricted) {
    delete next[provider.id];
    return next;
  }
  if (selection === "default") {
    delete next[provider.id];
  } else if (selection === "off") {
    next[provider.id] = null;
  } else {
    next[provider.id] = selection;
  }
  return next;
}

/** Materialize a semantic edit intent against the latest server values. Used
 *  both for initial saves and for rebuilding a conflicted write on 409 so a
 *  retry never overwrites another device's edits to unrelated Providers. */
export function buildRolloverChanges(
  baseValues: ContextRolloverValues,
  providers: readonly ContextRolloverProviderChoice[],
  intent: RolloverIntent,
): Partial<ContextRolloverValues> {
  switch (intent.kind) {
    case "enabled":
      return { enabled: intent.enabled };
    case "min_context_tokens":
      return { min_context_tokens: intent.min_context_tokens };
    case "reset_defaults":
      return {
        enabled: true,
        min_context_tokens: DEFAULT_MIN_CONTEXT_TOKENS,
        idle_minutes: {},
      };
    case "provider_idle": {
      const provider = providers.find((entry) => entry.id === intent.providerId) ?? {
        id: intent.providerId,
        name: intent.providerId,
        builtin_idle_minutes: null,
        recommended_idle_minutes: null,
        policy_restricted: intent.providerId === "antigravity",
        restriction: null,
        keepalive_independent: false,
      };
      return {
        idle_minutes: withProviderIdleSelection(
          baseValues,
          providers,
          provider,
          intent.selection,
        ),
      };
    }
  }
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

  const defaultLabel =
    provider.builtin_idle_minutes !== null
      ? `Use default (${formatIdleMinutesLabel(provider.builtin_idle_minutes)})`
      : "Use default (off)";
  const options: IdleChoiceOption[] = [
    { value: "default", label: defaultLabel },
    { value: "off", label: "Off (never renew)" },
  ];
  for (const minutes of sorted) {
    const base = formatIdleMinutesLabel(minutes);
    let suffix = "";
    if (minutes === provider.builtin_idle_minutes) {
      suffix = " · built-in";
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
    return (
      provider.restriction ??
      "Automatic context renewal is disabled for this Provider by host policy."
    );
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
    parts.push("Off by default");
  }
  if (provider.keepalive_independent) {
    parts.push("Independent of paid DeepSeek keepalive (off by default)");
  }
  return parts.join(" · ");
}

export function hasCustomRolloverPreferences(values: ContextRolloverValues): boolean {
  return (
    values.enabled !== true ||
    values.min_context_tokens !== DEFAULT_MIN_CONTEXT_TOKENS ||
    Object.keys(values.idle_minutes).length > 0
  );
}

export interface PendingRolloverWrite {
  datasetId: string;
  operation: string;
  scope: "context_rollover";
  expected_revision: number;
  intent: RolloverIntent;
  changes: Partial<ContextRolloverValues>;
}

/** Reconcile a freshly loaded snapshot against the previous state without ever
 *  comparing revision numbers across different Service datasets. */
export function reconcileLoadedRolloverPayload(
  old: ContextRolloverPreferencesPayload | undefined,
  next: ContextRolloverPreferencesPayload,
): { payload: ContextRolloverPreferencesPayload; datasetChanged: boolean } {
  if (!old) return { payload: next, datasetChanged: false };
  if (old.dataset.dataset_id !== next.dataset.dataset_id) {
    return { payload: next, datasetChanged: true };
  }
  return {
    payload: next.preferences.revision >= old.preferences.revision ? next : old,
    datasetChanged: false,
  };
}

/** Rebuild a conflicted pending write from the server's latest snapshot while
 *  staying bound to the original dataset and semantic intent. */
export function rebuildConflictedRolloverWrite(
  input: PendingRolloverWrite,
  currentSnapshot: ContextRolloverSnapshot,
  providers: readonly ContextRolloverProviderChoice[],
  nextOperation: string,
): PendingRolloverWrite {
  const normalized = withContextRolloverDefaults(currentSnapshot);
  return {
    datasetId: input.datasetId,
    operation: nextOperation,
    scope: "context_rollover",
    expected_revision: normalized.revision,
    intent: input.intent,
    changes: buildRolloverChanges(normalized.values, providers, input.intent),
  };
}

interface RolloverPutResult {
  status: number;
  ok: boolean;
  preferences?: ContextRolloverSnapshot;
  current?: ContextRolloverSnapshot;
  error?: { code: string };
}

async function putRolloverPreferences(input: PendingRolloverWrite): Promise<RolloverPutResult> {
  const response = await fetch("/api/preferences", {
    method: "PUT",
    signal: productSessionSignal(),
    headers: {
      "Content-Type": "application/json",
      "x-cowboy-dataset": input.datasetId,
    },
    body: JSON.stringify({
      operation: input.operation,
      scope: input.scope,
      expected_revision: input.expected_revision,
      changes: input.changes,
    }),
  });
  const body = (await response.json()) as {
    preferences?: ContextRolloverSnapshot;
    current?: ContextRolloverSnapshot;
    error?: { code: string };
  };
  return { status: response.status, ok: response.ok, ...body };
}

function applySnapshotForDataset(
  old: ContextRolloverPreferencesPayload | undefined,
  datasetId: string,
  snapshot: ContextRolloverSnapshot,
): ContextRolloverPreferencesPayload | undefined {
  if (!old || old.dataset.dataset_id !== datasetId || snapshot.revision < old.preferences.revision) {
    return old;
  }
  return { ...old, preferences: snapshot };
}

export function useContextRolloverPreferences(connected: boolean) {
  const [data, setData] = useState<ContextRolloverPreferencesPayload>();
  const [error, setError] = useState<string>();
  const [loadError, setLoadError] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [pending, setPending] = useState<PendingRolloverWrite>();
  const alive = useRef(false);
  const dataRef = useRef(data);
  dataRef.current = data;
  const pendingRef = useRef(pending);
  pendingRef.current = pending;
  const loading = useRef(0);
  const writing = useRef(false);

  const load = useCallback(async () => {
    const sequence = ++loading.current;
    try {
      const response = await fetch("/api/preferences/context_rollover", {
        cache: "no-store",
        signal: productSessionSignal(),
      });
      if (!response.ok) throw new Error("Could not load prompt-cache renewal preferences.");
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
      const reconciled = reconcileLoadedRolloverPayload(dataRef.current, normalized);
      setData(reconciled.payload);
      if (reconciled.datasetChanged && pendingRef.current) {
        setPending(undefined);
        setError("Service dataset changed. Reloaded settings and cleared the unsaved edit.");
      }
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
    const onOnline = () => void load();
    window.addEventListener(PREFERENCES_CHANGED, refresh);
    window.addEventListener("online", onOnline);
    return () => {
      alive.current = false;
      ++loading.current;
      window.removeEventListener(PREFERENCES_CHANGED, refresh);
      window.removeEventListener("online", onOnline);
    };
  }, [load, connected]);

  const commit = async (input: PendingRolloverWrite): Promise<void> => {
    const current = dataRef.current;
    if (!current || writing.current || productSessionSignal().aborted) return;
    if (current.dataset.dataset_id !== input.datasetId) {
      setPending(undefined);
      setError("Service dataset changed. Reloaded settings and cleared the unsaved edit.");
      void load();
      return;
    }
    writing.current = true;
    setSaving(true);
    setPending(input);
    setError(undefined);
    try {
      const result = await putRolloverPreferences(input);
      if (!alive.current || productSessionSignal().aborted) return;
      if (result.status === 409 && result.error?.code === "dataset_changed") {
        setPending(undefined);
        setError("Service dataset changed. Reloaded settings; apply your edit again.");
        void load();
        return;
      }
      if (result.status === 409 && result.current) {
        const latest = withContextRolloverDefaults(result.current);
        setData((old) => applySnapshotForDataset(old, input.datasetId, latest));
        setError("Prompt-cache settings changed elsewhere. Retry to apply your change or discard it.");
        setPending(rebuildConflictedRolloverWrite(input, latest, current.providers, crypto.randomUUID()));
        return;
      }
      if (!result.ok || !result.preferences) {
        throw new Error(result.error?.code ?? "Could not save prompt-cache settings");
      }
      const saved = withContextRolloverDefaults(result.preferences);
      setData((old) => applySnapshotForDataset(old, input.datasetId, saved));
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
    apply: (intent: RolloverIntent) =>
      data && !pending
        ? commit({
            datasetId: data.dataset.dataset_id,
            operation: crypto.randomUUID(),
            scope: "context_rollover",
            expected_revision: data.preferences.revision,
            intent,
            changes: buildRolloverChanges(data.preferences.values, data.providers, intent),
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
