import { useCallback, useEffect, useState } from "react";

/** Agent tools the Controller enforces: per agent kind, per-session overrides. */
export interface CallTarget {
  agent: string;
  preset?: string;
}

export interface CallsPolicy {
  enabled: boolean;
  targets: CallTarget[];
  default: string;
  max_concurrent: number;
  max_per_session: number;
}

export interface AgentTools {
  schema: 1;
  matrix: { tools: boolean; recall: boolean };
  calls: CallsPolicy;
}

export interface SessionToolsOverride {
  schema: 1;
  matrix?: { tools?: boolean; recall?: boolean };
  calls?: { enabled?: boolean; targets?: CallTarget[]; default?: string };
}

export interface ToolPreset {
  id: string;
  name: string;
  detail: string;
  is_default: boolean;
}

export interface ToolsCatalog {
  call_targets: { agent: string; presets: ToolPreset[] }[];
}

export interface SessionToolsView {
  session_id: string;
  agent: string;
  defaults: AgentTools;
  defaults_customized: boolean;
  override: SessionToolsOverride;
  effective: AgentTools;
  catalog: ToolsCatalog;
}

export interface AgentToolsList {
  agents: { agent: string; settings: AgentTools; customized: boolean }[];
  catalog: ToolsCatalog;
}

export const AUTO = "auto";
export const CONCURRENCY_CHOICES = [1, 2, 4, 8] as const;
export const SESSION_LIMIT_CHOICES = [16, 32, 64, 128, 256] as const;

export function agentLabel(agent: string): string {
  if (agent === "codex") return "Codex";
  if (agent === "claude-code") return "Claude";
  if (agent === AUTO) return "Auto";
  return agent.split("-").map((part) =>
    part.charAt(0).toUpperCase() + part.slice(1)
  ).join(" ");
}

/** Replace, add or remove one agent from a target list, keeping its order. */
export function withTarget(
  targets: CallTarget[],
  agent: string,
  allowed: boolean,
  preset?: string,
): CallTarget[] {
  const existing = targets.find((target) => target.agent === agent);
  if (!allowed) return targets.filter((target) => target.agent !== agent);
  const next: CallTarget = preset === undefined ? { agent } : { agent, preset };
  return existing
    ? targets.map((target) => target.agent === agent ? next : target)
    : [...targets, next];
}

/** Keep a default that still names an allowed agent. */
export function normalizedDefault(policy: CallsPolicy): string {
  return policy.default === AUTO ||
      policy.targets.some((target) => target.agent === policy.default)
    ? policy.default
    : AUTO;
}

function sameTargets(left: CallTarget[], right: CallTarget[]): boolean {
  return JSON.stringify(left.map((target) => [target.agent, target.preset])) ===
    JSON.stringify(right.map((target) => [target.agent, target.preset]));
}

/** The minimal session override producing `next` over the agent defaults:
 * fields equal to the defaults are inherited, not copied. */
export function overrideFor(
  defaults: AgentTools,
  next: AgentTools,
): SessionToolsOverride {
  const override: SessionToolsOverride = { schema: 1 };
  const matrix: NonNullable<SessionToolsOverride["matrix"]> = {};
  if (next.matrix.tools !== defaults.matrix.tools) {
    matrix.tools = next.matrix.tools;
  }
  if (next.matrix.recall !== defaults.matrix.recall) {
    matrix.recall = next.matrix.recall;
  }
  if (Object.keys(matrix).length > 0) override.matrix = matrix;
  const calls: NonNullable<SessionToolsOverride["calls"]> = {};
  if (next.calls.enabled !== defaults.calls.enabled) {
    calls.enabled = next.calls.enabled;
  }
  if (!sameTargets(next.calls.targets, defaults.calls.targets)) {
    calls.targets = next.calls.targets;
  }
  if (normalizedDefault(next.calls) !== normalizedDefault(defaults.calls)) {
    calls.default = normalizedDefault(next.calls);
  }
  if (Object.keys(calls).length > 0) override.calls = calls;
  return override;
}

export function overridden(override: SessionToolsOverride): boolean {
  return override.matrix !== undefined || override.calls !== undefined;
}

async function json<T>(response: Response, what: string): Promise<T> {
  if (!response.ok) throw new Error(`${what} (${response.status})`);
  return await response.json() as T;
}

export async function fetchSessionTools(
  session: string,
): Promise<SessionToolsView> {
  return await json(
    await fetch(`/api/sessions/${encodeURIComponent(session)}/tools`),
    "Tools unavailable",
  );
}

export async function saveSessionTools(
  session: string,
  override: SessionToolsOverride,
): Promise<SessionToolsView> {
  return await json(
    await fetch(`/api/sessions/${encodeURIComponent(session)}/tools`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(override),
    }),
    "Tools were not saved",
  );
}

export async function fetchAgentTools(): Promise<AgentToolsList> {
  return await json(await fetch("/api/agent-tools"), "Agent tools unavailable");
}

export async function saveAgentTools(
  agent: string,
  settings: AgentTools | null,
): Promise<{ agent: string; settings: AgentTools; customized: boolean }> {
  return await json(
    await fetch(`/api/agent-tools/${encodeURIComponent(agent)}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ settings }),
    }),
    "Defaults were not saved",
  );
}

/** Load-and-save state for one remote document; edits apply optimistically
 * and the server's answer replaces them. */
export function useRemoteDocument<T>(
  load: () => Promise<T>,
  key: string,
): {
  value: T | null;
  error: string | null;
  saving: boolean;
  save: (write: () => Promise<T>, optimistic?: T) => void;
} {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    let live = true;
    setValue(null);
    setError(null);
    load().then(
      (loaded) => live && setValue(loaded),
      (failure: unknown) =>
        live && setError(String((failure as Error).message ?? failure)),
    );
    return () => {
      live = false;
    };
    // `key` names the document; `load` closes over the same identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const save = useCallback((write: () => Promise<T>, optimistic?: T) => {
    const previous = value;
    if (optimistic !== undefined) setValue(optimistic);
    setSaving(true);
    write().then(
      (saved) => {
        setValue(saved);
        setError(null);
      },
      (failure: unknown) => {
        setValue(previous);
        setError(String((failure as Error).message ?? failure));
      },
    ).finally(() => setSaving(false));
  }, [value]);
  return { value, error, saving, save };
}
