import { useEffect, useRef, useState } from "react";
import type { SessionMeta } from "./protocol";

/** A parent-scoped managed call summary. Execution state, a structured review
 * result and the calling workflow's disposition of findings are separate
 * facts; only the first two come from Cowboy. */
export type ManagedCallState =
  | "queued"
  | "starting"
  | "running"
  | "waiting_input"
  | "stopping"
  | "completed"
  | "failed"
  | "cancelled";

export interface ManagedCallSummary {
  call_id: string;
  request_id: string;
  provider: string;
  purpose: string;
  labels: Record<string, string>;
  placement: {
    parent_session_id: string;
    machine_id: string;
    workspace_id: string;
  };
  child_session_id: string;
  state: ManagedCallState;
  created_at_ms: number;
  updated_at_ms: number;
  started_at_ms?: number | null;
  finished_at_ms?: number | null;
  configuration?: Record<string, string> | null;
  preset?: string | null;
  selection?: string | null;
  input_revision: string | null;
  has_result: boolean;
  runtime_machine_id: string | null;
  provider_version: string | null;
  cancel_requested: boolean;
  error: { code: string; detail?: string } | null;
  verdict?: string;
  finding_count?: number;
}

export interface ManagedCallResult {
  text?: string;
  truncated?: boolean;
  structured?: unknown;
  stop_reason?: string;
}

export interface ManagedCallDetail extends
  Omit<
    ManagedCallSummary,
    "verdict" | "finding_count" | "has_result" | "cancel_requested"
  > {
  result: ManagedCallResult | null;
  cancel_requested_at_ms?: number | null;
  provider_generation_digest?: string | null;
}

const ACTIVE: ReadonlySet<ManagedCallState> = new Set([
  "queued",
  "starting",
  "running",
  "waiting_input",
  "stopping",
]);

export function callActive(state: ManagedCallState): boolean {
  return ACTIVE.has(state);
}

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/**
 * The parent of a managed child conversation, read from its binding: a
 * target-local child's own record, or the `managed` constraint of a child
 * whose Agent runtime is on another Machine.
 */
export function managedChildParent(
  session: SessionMeta | undefined,
): string | null {
  const binding = object(session?.execution_binding);
  if (binding?.schema !== 1) return null;
  if (binding.phase === "managed_child") {
    return typeof binding.parent_session_id === "string"
      ? binding.parent_session_id
      : null;
  }
  const managed = object(binding.managed);
  return typeof managed?.parent_session_id === "string"
    ? managed.parent_session_id
    : null;
}

export function isManagedChild(session: SessionMeta | undefined): boolean {
  return managedChildParent(session) !== null;
}

export function callStateLabel(
  call: Pick<ManagedCallSummary, "state" | "cancel_requested">,
): string {
  switch (call.state) {
    case "queued":
      return call.cancel_requested ? "Stopping" : "Queued";
    case "starting":
      return "Starting";
    case "running":
      return call.cancel_requested ? "Stopping" : "Running";
    case "waiting_input":
      return "Needs input";
    case "stopping":
      return "Stopping";
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
  }
}

export type CallTone = "active" | "success" | "error" | "neutral";

export function callTone(state: ManagedCallState): CallTone {
  if (callActive(state)) return "active";
  if (state === "completed") return "success";
  if (state === "failed") return "error";
  return "neutral";
}

/** Labels are caller-owned display metadata; never authority or workflow. */
export function callTitle(
  call: Pick<ManagedCallSummary, "labels" | "purpose" | "request_id">,
): string {
  const aspect = call.labels["aspect"];
  const round = call.labels["round"];
  const parts = [
    aspect ? aspect.replace(/[-_]/g, " ") : null,
    round ? `round ${round}` : null,
  ].filter((value): value is string => value !== null);
  if (parts.length > 0) return parts.join(" · ");
  return call.labels["title"] ?? call.purpose.replace(/_/g, " ");
}

/** Persisted observations, never today's mutable Provider catalog. */
export function callModelLabel(call: Pick<ManagedCallSummary, "configuration" | "preset">): string {
  const model = call.configuration?.model;
  const effort = call.configuration?.reasoning_effort ?? call.configuration?.effort;
  if (model) return [model, effort].filter(Boolean).join(" · ");
  return call.preset ? `Selected: ${call.preset}` : "Model not recorded";
}

/** Only the call's recorded values describe its execution, including failed
 * starts. Never fill historical gaps from a mutable Provider preset. */
export function callExecutionParameters(call: Pick<ManagedCallSummary, "configuration" | "preset">): { label: string; value: string }[] {
  const config = call.configuration;
  const permission = ["permission_mode", "sandbox_mode", "approval_policy"]
    .flatMap((key) => config?.[key] ? [`${key}: ${config[key]}`] : []);
  return [
    { label: "Model", value: config?.model || "Not recorded" },
    { label: "Reasoning", value: config?.reasoning_effort || config?.effort || "Not recorded" },
    { label: "Permission", value: permission.join(" · ") || "Not recorded" },
    ...(config?.session_mode || config?.mode ? [{ label: "Mode", value: config.session_mode || config.mode! }] : []),
    ...(call.preset ? [{ label: "Preset", value: call.preset }] : []),
  ];
}

/** Compact titles omit transport IDs, but repeated tasks still need identity.
 * Number by creation order, never status or the incoming poll order. */
export function callPreviewTitles(calls: readonly ManagedCallSummary[]): Map<string, string> {
  const titles = new Map<string, string>();
  const groups = new Map<string, ManagedCallSummary[]>();
  for (const call of calls) {
    const title = (call.labels["title"]?.trim() || call.labels["aspect"]?.trim() || call.purpose)
      .replace(/[-_]/g, " ");
    titles.set(call.call_id, title);
    const key = title.toLocaleLowerCase();
    const group = groups.get(key) ?? [];
    group.push(call);
    groups.set(key, group);
  }
  for (const group of groups.values()) {
    if (group.length < 2) continue;
    group.sort((a, b) => a.created_at_ms - b.created_at_ms || a.call_id.localeCompare(b.call_id));
    group.forEach((call, index) => titles.set(call.call_id, `${titles.get(call.call_id)} · ${index + 1}`));
  }
  return titles;
}

export function callGroup(
  call: Pick<ManagedCallSummary, "labels">,
): string | null {
  return call.labels["group"] ?? call.labels["task"] ?? null;
}

export function machineName(id: string | null | undefined): string {
  if (!id) return "Unknown";
  return id.toLowerCase() === "ovh"
    ? "OVH"
    : id.charAt(0).toUpperCase() + id.slice(1);
}

export function providerLabel(provider: string): string {
  return provider === "claude-code"
    ? "Claude"
    : provider === "codex"
    ? "Codex"
    : provider;
}

export function elapsedLabel(fromMs: number, toMs: number): string {
  const seconds = Math.max(0, Math.round((toMs - fromMs) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** Keep preview and detail ages current without updating hidden views. */
export function useCallClock(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const update = (): void => {
      if (document.visibilityState !== "hidden") setNow(Date.now());
    };
    const timer = globalThis.setInterval(update, 30_000);
    document.addEventListener("visibilitychange", update);
    return () => {
      globalThis.clearInterval(timer);
      document.removeEventListener("visibilitychange", update);
    };
  }, []);
  return now;
}

/** Shared compact relative start time for call previews and detail headers. */
export function callAgeLabel(atMs: number, nowMs: number): string {
  const minutes = Math.max(0, Math.floor((nowMs - atMs) / 60_000));
  if (minutes < 1) return "now";
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor(minutes / 60) % 24;
  const remainingMinutes = minutes % 60;
  const parts = [
    days ? `${days}d` : "",
    hours ? `${hours}h` : "",
    !days && remainingMinutes ? `${remainingMinutes}m` : "",
  ].filter(Boolean);
  return `${parts.join(" ")} ago`;
}

/** When a call started, as compactly as it stays unambiguous: the time
 * today, "Yesterday" or the date before that, and the year only when it
 * differs. Paired with its duration, it also places the call's end. */
export function startedLabel(atMs: number, nowMs: number = Date.now()): string {
  const at = new Date(atMs);
  const now = new Date(nowMs);
  const time = at.toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
  });
  const day = (date: Date) =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round((day(now) - day(at)) / 86_400_000);
  if (days === 0) return time;
  if (days === 1) return `Yesterday ${time}`;
  const date = at.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    ...(at.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
  return `${date} ${time}`;
}

/** Exact start and end for a hover or long-press title. */
export function callTimesTitle(
  call: Pick<ManagedCallSummary, "created_at_ms" | "started_at_ms" | "finished_at_ms" | "state">,
): string {
  const created = `Created ${new Date(call.created_at_ms).toLocaleString()}`;
  const started = call.started_at_ms == null ? "Start not recorded" : `Started ${new Date(call.started_at_ms).toLocaleString()}`;
  const finished = call.finished_at_ms == null ? (callActive(call.state) ? "In progress" : "Finish not recorded") : `Finished ${new Date(call.finished_at_ms).toLocaleString()}`;
  return `${created} · ${started} · ${finished}`;
}

export function shortRevision(revision: string | null | undefined): string {
  if (!revision) return "—";
  return revision.replace(/^sha256:/, "").slice(0, 12);
}

export interface CallsOverview {
  total: number;
  active: number;
  completed: number;
  failed: number;
  activeTitle: string | null;
  findings: number;
}

export function callsOverview(
  calls: readonly ManagedCallSummary[],
): CallsOverview {
  const active = calls.filter((call) => callActive(call.state));
  return {
    total: calls.length,
    active: active.length,
    completed: calls.filter((call) => call.state === "completed").length,
    failed: calls.filter((call) => call.state === "failed").length,
    activeTitle: active[0] ? callTitle(active[0]) : null,
    findings: calls.reduce((sum, call) => sum + (call.finding_count ?? 0), 0),
  };
}

/** Findings from a structured review result, when the caller's schema used
 * the common `findings` shape. Anything else is shown as raw JSON. */
export interface ReviewFinding {
  title: string;
  body: string | null;
  severity: string | null;
  location: string | null;
}

export function reviewFindings(structured: unknown): ReviewFinding[] | null {
  const root = object(structured);
  if (!root || !Array.isArray(root.findings)) return null;
  return root.findings.map((value) => {
    const finding = object(value) ?? {};
    const text = (key: string): string | null =>
      typeof finding[key] === "string" ? finding[key] as string : null;
    const file = text("file") ?? text("path");
    const line = typeof finding.line_start === "number"
      ? finding.line_start
      : typeof finding.line === "number"
      ? finding.line
      : null;
    return {
      title: text("title") ?? text("summary") ?? "Finding",
      body: text("body") ?? text("details") ?? text("recommendation"),
      severity: text("severity") ?? text("priority"),
      location: file ? (line !== null ? `${file}:${line}` : file) : null,
    };
  });
}

export function reviewSummary(structured: unknown): string | null {
  const root = object(structured);
  return root && typeof root.summary === "string" ? root.summary : null;
}

export function reviewVerdict(structured: unknown): string | null {
  const root = object(structured);
  return root && typeof root.verdict === "string" ? root.verdict : null;
}

export async function fetchCalls(
  parent: string,
  signal?: AbortSignal,
): Promise<ManagedCallSummary[]> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(parent)}/calls`,
    { signal: signal ?? null },
  );
  if (response.status === 404) return [];
  if (!response.ok) throw new Error(`Calls unavailable (${response.status})`);
  const body = await response.json() as { calls?: ManagedCallSummary[] };
  return Array.isArray(body.calls) ? body.calls : [];
}

export async function fetchCall(
  parent: string,
  call: string,
  signal?: AbortSignal,
): Promise<ManagedCallDetail> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(parent)}/calls/${
      encodeURIComponent(call)
    }`,
    { signal: signal ?? null },
  );
  if (!response.ok) throw new Error(`Call unavailable (${response.status})`);
  const body = await response.json() as { call: ManagedCallDetail };
  return body.call;
}

export async function cancelCall(parent: string, call: string): Promise<void> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(parent)}/calls/${
      encodeURIComponent(call)
    }/cancel`,
    { method: "POST" },
  );
  if (!response.ok) {
    throw new Error(`Stop was not recorded (${response.status})`);
  }
}

export async function dismissCalls(parent: string, ids: string[], dismissed = true): Promise<string[]> {
  const response = await fetch(`/api/sessions/${encodeURIComponent(parent)}/calls/dismiss`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ call_ids: ids, dismissed }),
  });
  if (!response.ok) throw new Error(`Calls could not be cleared (${response.status}); retry safely`);
  const result = await response.json() as { call_ids: string[] };
  return result.call_ids;
}

/** Poll one parent's calls. Active calls refresh quickly; settled lists slowly.
 * The last observation is kept across network failures with its age. */
export function useManagedCalls(parent: string | null): {
  calls: ManagedCallSummary[];
  observedAt: number | null;
  error: string | null;
  refresh: () => void;
} {
  const [calls, setCalls] = useState<ManagedCallSummary[]>([]);
  const [observedAt, setObservedAt] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const activeRef = useRef(false);
  useEffect(() => {
    setCalls([]);
    setObservedAt(null);
    setError(null);
  }, [parent]);
  useEffect(() => {
    if (!parent) return;
    const controller = new AbortController();
    let timer: number | undefined;
    const load = (): void => {
      if (
        typeof document !== "undefined" && document.visibilityState === "hidden"
      ) {
        timer = globalThis.setTimeout(load, 5_000);
        return;
      }
      void fetchCalls(parent, controller.signal).then((next) => {
        if (controller.signal.aborted) return;
        setCalls(next);
        setObservedAt(Date.now());
        setError(null);
        activeRef.current = next.some((call) => callActive(call.state));
      }).catch((reason: unknown) => {
        if (!controller.signal.aborted) {
          setError(
            reason instanceof Error ? reason.message : "Calls unavailable",
          );
        }
      }).finally(() => {
        if (!controller.signal.aborted) {
          timer = globalThis.setTimeout(
            load,
            activeRef.current ? 2_000 : 20_000,
          );
        }
      });
    };
    load();
    return (): void => {
      controller.abort();
      if (timer !== undefined) globalThis.clearTimeout(timer);
    };
  }, [parent, tick]);
  return {
    calls,
    observedAt,
    error,
    refresh: (): void => setTick((value) => value + 1),
  };
}

export function useManagedCall(
  parent: string | null,
  call: string | null,
  revision: number,
): {
  detail: ManagedCallDetail | null;
  error: string | null;
} {
  const [detail, setDetail] = useState<ManagedCallDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setDetail(null);
    setError(null);
  }, [parent, call]);
  useEffect(() => {
    if (!parent || !call) return;
    const controller = new AbortController();
    void fetchCall(parent, call, controller.signal).then((value) => {
      setDetail(value);
      setError(null);
    }).catch((reason: unknown) => {
      if (!controller.signal.aborted) {
        setError(reason instanceof Error ? reason.message : "Call unavailable");
      }
    });
    return (): void => controller.abort();
  }, [parent, call, revision]);
  return { detail, error };
}
