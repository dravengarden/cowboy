import { timeRangeQuery, type ObservabilityTimeRange } from "./observabilityTimeRange";

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
