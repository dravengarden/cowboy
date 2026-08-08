import {
  timeRangeQuery,
  type ObservabilityTimeRange,
  validTimeRange,
} from "./observabilityTimeRange";

export type DiagnosticLogKind =
  | "session_error"
  | "provider_error"
  | "cache_anomaly"
  | "automation";

export type DiagnosticLogSeverity =
  | "info"
  | "warning"
  | "error"
  | "critical";

export type DiagnosticLogState =
  | "active"
  | "recovered"
  | "failed"
  | "succeeded"
  | "observed"
  | "scheduled"
  | "started"
  | "retrying"
  | "unknown"
  | "cancelled";

export type DiagnosticLogAgent = "codex" | "claude";

export interface DiagnosticLogFilters {
  kinds: DiagnosticLogKind[];
  severities: DiagnosticLogSeverity[];
  states: DiagnosticLogState[];
  agents: DiagnosticLogAgent[];
  timeRange: ObservabilityTimeRange;
}

export interface DiagnosticLogSummary {
  id: string;
  occurred_at_ms: number;
  kind: DiagnosticLogKind;
  severity: DiagnosticLogSeverity;
  state: string;
  title: string;
  summary: string;
  session_ref?: string;
  provider?: string;
  agent?: string;
  model?: string;
  classification?: string;
}

export interface DiagnosticLogField {
  label: string;
  value: string;
  copyable: boolean;
}

export interface DiagnosticLogDetail {
  id: string;
  kind: DiagnosticLogSummary["kind"];
  occurred_at_ms: number;
  title: string;
  summary: string;
  sections: Array<{
    title: string;
    fields: DiagnosticLogField[];
  }>;
  evidence?: unknown;
}

export interface DiagnosticLogPage {
  items: DiagnosticLogSummary[];
  next_cursor?: string;
}

export const DEFAULT_DIAGNOSTIC_LOG_FILTERS: DiagnosticLogFilters = {
  kinds: [],
  severities: ["critical", "error"],
  states: [],
  agents: [],
  timeRange: { mode: "relative", amount: 7, unit: "day" },
};

const DIAGNOSTIC_LOG_FILTERS_KEY = "cowboy.diagnostic.logs.filters";
const DIAGNOSTIC_LOG_KINDS: readonly DiagnosticLogKind[] = [
  "session_error",
  "provider_error",
  "cache_anomaly",
  "automation",
];
const DIAGNOSTIC_LOG_SEVERITIES: readonly DiagnosticLogSeverity[] = [
  "info",
  "warning",
  "error",
  "critical",
];
const DIAGNOSTIC_LOG_STATES: readonly DiagnosticLogState[] = [
  "active",
  "recovered",
  "failed",
  "succeeded",
  "observed",
  "scheduled",
  "started",
  "retrying",
  "unknown",
  "cancelled",
];
const DIAGNOSTIC_LOG_AGENTS: readonly DiagnosticLogAgent[] = ["codex", "claude"];

export function cloneDiagnosticLogFilters(
  filters: DiagnosticLogFilters,
): DiagnosticLogFilters {
  return {
    kinds: [...filters.kinds],
    severities: [...filters.severities],
    states: [...filters.states],
    agents: [...filters.agents],
    timeRange: filters.timeRange.mode === "relative"
      ? { ...filters.timeRange }
      : { ...filters.timeRange },
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function selectedValues<T extends string>(
  value: unknown,
  allowed: readonly T[],
): T[] {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((item): item is T =>
    typeof item === "string" && allowed.includes(item as T)
  ))];
}

function storedTimeRange(value: unknown): ObservabilityTimeRange | undefined {
  if (!isRecord(value) || typeof value.mode !== "string") return undefined;
  if (value.mode === "relative" && typeof value.amount === "number" &&
    Number.isFinite(value.amount) && value.amount > 0 &&
    (value.unit === "minute" || value.unit === "hour" || value.unit === "day")) {
    const candidate = {
      mode: "relative",
      amount: Math.trunc(value.amount),
      unit: value.unit,
    } as const;
    return validTimeRange(candidate, 365 * 86_400_000) ? candidate : undefined;
  }
  if (value.mode === "absolute" && typeof value.fromMs === "number" &&
    typeof value.toMs === "number") {
    const candidate = { mode: "absolute", fromMs: value.fromMs, toMs: value.toMs } as const;
    return validTimeRange(candidate, 365 * 86_400_000) ? candidate : undefined;
  }
  return undefined;
}

export function parseDiagnosticLogFilters(value: unknown): DiagnosticLogFilters {
  if (!isRecord(value)) return cloneDiagnosticLogFilters(DEFAULT_DIAGNOSTIC_LOG_FILTERS);
  return {
    kinds: Array.isArray(value.kinds) ? selectedValues(value.kinds, DIAGNOSTIC_LOG_KINDS) : [],
    severities: Array.isArray(value.severities)
      ? selectedValues(value.severities, DIAGNOSTIC_LOG_SEVERITIES)
      : [...DEFAULT_DIAGNOSTIC_LOG_FILTERS.severities],
    states: Array.isArray(value.states) ? selectedValues(value.states, DIAGNOSTIC_LOG_STATES) : [],
    agents: Array.isArray(value.agents) ? selectedValues(value.agents, DIAGNOSTIC_LOG_AGENTS) : [],
    timeRange: storedTimeRange(value.timeRange) ?? { ...DEFAULT_DIAGNOSTIC_LOG_FILTERS.timeRange },
  };
}

function localStorageOrUndefined(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

export function loadDiagnosticLogFilters(): DiagnosticLogFilters {
  const storage = localStorageOrUndefined();
  if (!storage) return cloneDiagnosticLogFilters(DEFAULT_DIAGNOSTIC_LOG_FILTERS);
  try {
    const stored = storage.getItem(DIAGNOSTIC_LOG_FILTERS_KEY);
    return stored ? parseDiagnosticLogFilters(JSON.parse(stored)) : cloneDiagnosticLogFilters(DEFAULT_DIAGNOSTIC_LOG_FILTERS);
  } catch {
    return cloneDiagnosticLogFilters(DEFAULT_DIAGNOSTIC_LOG_FILTERS);
  }
}

export function persistDiagnosticLogFilters(filters: DiagnosticLogFilters): void {
  try {
    localStorageOrUndefined()?.setItem(
      DIAGNOSTIC_LOG_FILTERS_KEY,
      JSON.stringify(cloneDiagnosticLogFilters(filters)),
    );
  } catch {
    // Private or locked-down WebViews may deny storage; live filters still work.
  }
}

export function diagnosticLogUrl(
  filters: DiagnosticLogFilters,
  cursor?: string,
  now: number = Date.now(),
): string {
  const params = new URLSearchParams({ limit: "25" });
  if (filters.kinds.length > 0) params.set("kind", filters.kinds.join(","));
  if (filters.severities.length > 0) params.set("severity", filters.severities.join(","));
  if (filters.states.length > 0) params.set("state", filters.states.join(","));
  if (filters.agents.length > 0) params.set("agent", filters.agents.join(","));
  const range = timeRangeQuery(filters.timeRange, now);
  params.set("from_ms", range.from_ms);
  params.set("to_ms", range.to_ms);
  if (cursor) params.set("cursor", cursor);
  return `/api/logs?${params.toString()}`;
}

export function diagnosticKindLabel(kind: DiagnosticLogSummary["kind"]): string {
  switch (kind) {
    case "session_error":
      return "Session";
    case "provider_error":
      return "Provider";
    case "cache_anomaly":
      return "Cache";
    case "automation":
      return "Automation";
  }
}
