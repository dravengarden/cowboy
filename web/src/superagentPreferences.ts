import { useCallback, useEffect, useRef, useState } from "react";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";

export type SuperagentMode = "off" | "observe" | "coordinate";
export interface PatternSource {
  revision: string;
  profile: string;
  files: Record<string, string>;
}
export interface SuperagentProject {
  profile: string;
  coordinator: string;
  preset: "astra-max" | "fable-max";
  mode: SuperagentMode;
  max_followups: number;
  learning: boolean;
}
export interface Participation {
  project: string;
  mode: SuperagentMode | null;
  mandate: string;
}
export interface SuperagentConfig {
  sources: PatternSource[];
  projects: Record<string, SuperagentProject>;
  sessions: Record<string, Participation>;
}
interface Snapshot {
  preferences: {
    revision: number;
    values: { configuration: SuperagentConfig };
  };
  dataset: { dataset_id: string; user_id: string };
}
interface Update {
  operation: string;
  scope: "superagent";
  expected_revision: number;
  changes: { configuration: SuperagentConfig };
}

function saveError(code: unknown): string {
  const messages: Record<string, string> = {
    revision_conflict:
      "Settings changed on another device. Review the current values and try again.",
    coordinator_must_be_idle: "Wait for the coordinator to finish its turn and queued messages.",
    superagent_requires_selected_max_model:
      "Select the matching Astra Max or Fable Max preset in the coordinator session first.",
    coordinator_configuration_pending: "Wait for the coordinator’s model change to finish.",
    machine_scope_denied:
      "The coordinator and participating sessions must execute on the same Machine.",
    superagent_mandate_required: "Describe the existing task scope before enabling coordination.",
    superagent_procedure_missing:
      "The pattern library needs a supervisor and all of its referenced procedures.",
    session_not_available:
      "A selected session is unavailable. Check its owner, Trash and closing state.",
    invalid_superagent_project:
      "Use a unique project ID and a separate coordinator session for each project.",
  };
  return typeof code === "string" && messages[code]
    ? messages[code]
    : "Could not save Super-agent settings. Check the selected sessions and pattern bundle.";
}

/** Display-only discovery; the Controller owns strict TOML parsing/validation. */
export function sourceProfileId(source: PatternSource): string | undefined {
  return source.files[source.profile]
    ?.split("\n+++\n")[0]
    ?.match(/^id\s*=\s*"([A-Za-z0-9_.-]+)"\s*$/m)?.[1];
}

export function parsePatternBundle(text: string): PatternSource[] {
  if (new TextEncoder().encode(text).length > 800_000) {
    throw new Error("Pattern bundle is too large.");
  }
  const sources: unknown = JSON.parse(text);
  if (
    !Array.isArray(sources) ||
    sources.length > 32 ||
    sources.some(
      (source) =>
        !source ||
        typeof source.profile !== "string" ||
        typeof source.revision !== "string" ||
        !source.files ||
        typeof source.files !== "object" ||
        Array.isArray(source.files) ||
        Object.values(source.files).some((text) => typeof text !== "string") ||
        !sourceProfileId(source),
    )
  )
    throw new Error("Choose a valid pattern source bundle.");
  return sources;
}

export function useSuperagentSettings() {
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [error, setError] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [pending, setPending] = useState<Update>();
  const alive = useRef(false);
  const writing = useRef(false);
  const current = useRef(snapshot);
  current.current = snapshot;
  const load = useCallback(async () => {
    try {
      const response = await fetch("/api/preferences/superagent", {
        cache: "no-store",
        signal: productSessionSignal(),
      });
      if (!response.ok) throw new Error("Could not load Super-agent settings.");
      const next = (await response.json()) as Snapshot;
      if (!alive.current || productSessionSignal().aborted) return;
      if (next.dataset.user_id !== productSyncPrincipal()) {
        throw new Error("Account changed. Reload settings.");
      }
      setSnapshot((old) =>
        !old || next.preferences.revision >= old.preferences.revision ? next : old,
      );
    } catch (cause) {
      if (alive.current) setError(String(cause));
    }
  }, []);
  useEffect(() => {
    alive.current = true;
    void load();
    const changed = (event: Event) => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (!scopes || scopes.superagent) void load();
    };
    window.addEventListener(PREFERENCES_CHANGED, changed);
    window.addEventListener("online", load);
    return () => {
      alive.current = false;
      window.removeEventListener(PREFERENCES_CHANGED, changed);
      window.removeEventListener("online", load);
    };
  }, [load]);
  const commit = async (input: Update) => {
    const observed = current.current;
    if (writing.current || !observed) return;
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
          "X-Cowboy-Dataset": observed.dataset.dataset_id,
        },
        body: JSON.stringify(input),
      });
      const result = await response.json();
      if (!response.ok) {
        if (response.status < 500) setPending(undefined);
        if (response.status === 409) await load();
        throw new Error(saveError(result.error?.code));
      }
      if (!alive.current || productSessionSignal().aborted) return;
      setSnapshot((old) =>
        !old || result.preferences.revision >= old.preferences.revision
          ? { ...observed, preferences: result.preferences }
          : old,
      );
      setPending(undefined);
    } catch (cause) {
      if (alive.current) setError(String(cause));
    } finally {
      writing.current = false;
      if (alive.current) setSaving(false);
    }
  };
  return {
    config: snapshot?.preferences.values.configuration,
    error,
    saving,
    pending,
    save: (configuration: SuperagentConfig) =>
      snapshot && !pending
        ? commit({
            operation: crypto.randomUUID(),
            scope: "superagent",
            expected_revision: snapshot.preferences.revision,
            changes: { configuration },
          })
        : Promise.resolve(),
    retry: () => {
      if (pending) return commit(pending);
      setError(undefined);
      return load();
    },
  };
}
