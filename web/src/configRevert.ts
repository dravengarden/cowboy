import { useCallback, useEffect, useState } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import type { ConfigOption } from "./protocol";

/** Values an in-place agent change (model, reasoning, …) replaced, restorable
 *  from Session Settings until the next message is sent. Server-owned so
 *  every device shows the same Back. */
export interface ConfigRevert {
  revision: number;
  values: Record<string, string | boolean>;
}

const EMPTY: ConfigRevert = { revision: 0, values: {} };

function url(id: string, suffix = ""): string {
  return `/api/sessions/${encodeURIComponent(id)}/config-revert${suffix}`;
}

async function request(id: string, init?: RequestInit, suffix = ""): Promise<ConfigRevert> {
  const response = await fetch(url(id, suffix), {
    cache: "no-store",
    ...init,
    signal: productSessionSignal(),
  });
  const body = await response.text();
  if (!response.ok) throw new Error(body || `Request failed (${response.status})`);
  const value = body ? JSON.parse(body) as Partial<ConfigRevert> : {};
  return { revision: value.revision ?? 0, values: value.values ?? {} };
}

export function loadConfigRevert(id: string): Promise<ConfigRevert> {
  return request(id);
}

/** Re-apply the recorded values; refused once a message was sent. */
export function restoreConfigRevert(id: string): Promise<ConfigRevert> {
  return request(id, { method: "POST" }, "/restore");
}

/** "Opus · High": the recorded values in the agent's option order, named by
 *  the agent's own choice labels. Empty when nothing can be restored. */
export function configRevertSummary(
  values: ConfigRevert["values"],
  options: readonly ConfigOption[] | undefined,
): string {
  const ids = Object.keys(values);
  if (ids.length === 0) return "";
  const ordered = [
    ...(options ?? []).filter((option) => ids.includes(option.id)),
    ...ids
      .filter((id) => !(options ?? []).some((option) => option.id === id))
      .map((id) => ({ id, name: id, options: [] }) as Pick<ConfigOption, "id" | "name" | "options">),
  ];
  return ordered
    .map((option) => {
      const value = values[option.id]!;
      const choice = option.options.find((entry) => entry.value === value);
      if (choice) return choice.name;
      return typeof value === "boolean" ? `${option.name} ${value ? "on" : "off"}` : String(value);
    })
    .join(" · ");
}

export function useConfigRevert(id: string): {
  revert: ConfigRevert;
  restore: () => Promise<void>;
  restoring: boolean;
  error: string;
} {
  const [loaded, setLoaded] = useState<{ id: string; revert: ConfigRevert }>();
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    setLoaded(undefined);
    setError("");
    const refresh = () => {
      void loadConfigRevert(id)
        .then((revert) => {
          if (alive) setLoaded({ id, revert });
        })
        .catch(() => undefined);
    };
    const preference = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (!scopes || Object.hasOwn(scopes, `session_config_revert:${id}`)) refresh();
    };
    refresh();
    window.addEventListener(PREFERENCES_CHANGED, preference);
    window.addEventListener("focus", refresh);
    return () => {
      alive = false;
      window.removeEventListener(PREFERENCES_CHANGED, preference);
      window.removeEventListener("focus", refresh);
    };
  }, [id]);
  const restore = useCallback(async () => {
    setRestoring(true);
    setError("");
    try {
      setLoaded({ id, revert: await restoreConfigRevert(id) });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not go back");
      void loadConfigRevert(id).then((revert) => setLoaded({ id, revert })).catch(() => undefined);
    } finally {
      setRestoring(false);
    }
  }, [id]);
  return {
    revert: loaded?.id === id ? loaded.revert : EMPTY,
    restore,
    restoring,
    error,
  };
}
