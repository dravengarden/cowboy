import { useCallback, useEffect, useState } from "react";

/** Agent tools the Controller enforces: per agent kind, per-session overrides. */
export interface CallTarget {
  agent: string;
  preset?: string;
  /** A custom choice instead of a preset: values for the option ids presets
   * set (model, reasoning). Never together with `preset`. */
  options?: Record<string, string>;
}

/** What a target's calls run with: a preset, custom values, or neither. */
export type TargetChoice = Pick<CallTarget, "preset" | "options">;

export interface CallsPolicy {
  enabled: boolean;
  targets: CallTarget[];
  default: string;
  max_concurrent: number;
  max_per_session: number;
}

export interface AgentTools {
  schema: 1;
  /** Served by the runtime Machine's installed memory backend Plugin. */
  memory: { tools: boolean; recall: boolean };
  calls: CallsPolicy;
}

export interface SessionToolsOverride {
  schema: 1;
  memory?: { tools?: boolean; recall?: boolean };
  calls?: { enabled?: boolean; targets?: CallTarget[]; default?: string };
}

export interface ToolPreset {
  id: string;
  name: string;
  detail: string;
  is_default: boolean;
  values?: Record<string, string>;
}

/** One finer setting a target's presets set, with its choices. */
export interface ToolOption {
  id: string;
  name: string;
  choices: { value: string; name: string }[];
}

export interface ToolsCatalog {
  call_targets: {
    agent: string;
    presets: ToolPreset[];
    options?: ToolOption[];
  }[];
}

/** The values a choice sets: its preset's, or its custom ones. */
export function choiceValues(
  choice: TargetChoice | undefined,
  presets: readonly ToolPreset[],
): Record<string, string> {
  if (choice?.options) return { ...choice.options };
  const preset = presets.find((candidate) => candidate.id === choice?.preset);
  return { ...(preset?.values ?? {}) };
}

/** A choice from values: the preset that sets exactly them, else custom. */
export function choiceFor(
  values: Record<string, string>,
  presets: readonly ToolPreset[],
): TargetChoice {
  const keys = Object.keys(values).sort();
  if (keys.length === 0) return {};
  const same = presets.find((preset) => {
    const own = preset.values ?? {};
    const ownKeys = Object.keys(own).sort();
    return ownKeys.length === keys.length &&
      ownKeys.every((key, index) =>
        key === keys[index] && own[key] === values[key]
      );
  });
  return same ? { preset: same.id } : { options: values };
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

/** Agents of one family review alike; an agent never calls its own family
 * (mirrors `same_family` in src/server/agent_tools.rs). */
export function sameFamily(left: string, right: string): boolean {
  const family = (agent: string): string =>
    ["claude", "codex"].find((name) =>
      agent === name || agent.startsWith(`${name}-`)
    ) ?? agent;
  return family(left) === family(right);
}

/** The catalog targets an agent of `caller`'s kind may call. */
export function callableTargets(
  caller: string,
  catalog: ToolsCatalog,
): ToolsCatalog["call_targets"] {
  return catalog.call_targets.filter((target) =>
    !sameFamily(target.agent, caller)
  );
}

/** Replace, add or remove one agent from a target list, keeping its order. */
export function withTarget(
  targets: CallTarget[],
  agent: string,
  allowed: boolean,
  choice?: string | TargetChoice,
): CallTarget[] {
  const existing = targets.find((target) => target.agent === agent);
  if (!allowed) return targets.filter((target) => target.agent !== agent);
  const picked: TargetChoice = typeof choice === "string"
    ? { preset: choice }
    : choice ?? {};
  const next: CallTarget = picked.preset !== undefined
    ? { agent, preset: picked.preset }
    : picked.options && Object.keys(picked.options).length > 0
    ? { agent, options: picked.options }
    : { agent };
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
  const key = (target: CallTarget): unknown[] => [
    target.agent,
    target.preset,
    target.options
      ? Object.entries(target.options).sort(([a], [b]) => a.localeCompare(b))
      : undefined,
  ];
  return JSON.stringify(left.map(key)) === JSON.stringify(right.map(key));
}

/** The minimal session override producing `next` over the agent defaults:
 * fields equal to the defaults are inherited, not copied. */
export function overrideFor(
  defaults: AgentTools,
  next: AgentTools,
): SessionToolsOverride {
  const override: SessionToolsOverride = { schema: 1 };
  const memory: NonNullable<SessionToolsOverride["memory"]> = {};
  if (next.memory.tools !== defaults.memory.tools) {
    memory.tools = next.memory.tools;
  }
  if (next.memory.recall !== defaults.memory.recall) {
    memory.recall = next.memory.recall;
  }
  if (Object.keys(memory).length > 0) override.memory = memory;
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
  return override.memory !== undefined || override.calls !== undefined;
}

/** Controllers before session extensions name the memory switches `matrix`. */
export function withMemoryKey<T>(value: T): T {
  if (Array.isArray(value)) return value.map(withMemoryKey) as T;
  if (value === null || typeof value !== "object") return value;
  const entries = Object.entries(value as Record<string, unknown>).map((
    [key, item],
  ) => [key === "matrix" ? "memory" : key, withMemoryKey(item)] as const);
  return Object.fromEntries(entries) as T;
}

async function json<T>(response: Response, what: string): Promise<T> {
  if (!response.ok) throw new Error(`${what} (${response.status})`);
  return withMemoryKey(await response.json() as T);
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
