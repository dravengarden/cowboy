export type DiagnosticLogKind =
  | "all"
  | "session_error"
  | "provider_error"
  | "cache_anomaly"
  | "automation";

export type DiagnosticLogSeverity =
  | "all"
  | "info"
  | "warning"
  | "error"
  | "critical";

export type DiagnosticLogState =
  | "all"
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

export type DiagnosticLogAgent = "all" | "codex" | "claude";
export type DiagnosticLogWindow = "1h" | "24h" | "7d" | "30d";

export interface DiagnosticLogFilters {
  kind: DiagnosticLogKind;
  severity: DiagnosticLogSeverity;
  state: DiagnosticLogState;
  agent: DiagnosticLogAgent;
  window: DiagnosticLogWindow;
}

export interface DiagnosticLogSummary {
  id: string;
  occurred_at_ms: number;
  kind: Exclude<DiagnosticLogKind, "all">;
  severity: Exclude<DiagnosticLogSeverity, "all">;
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
  kind: "all",
  severity: "all",
  state: "all",
  agent: "all",
  window: "7d",
};

export function diagnosticLogUrl(
  filters: DiagnosticLogFilters,
  cursor?: string,
): string {
  const params = new URLSearchParams({
    kind: filters.kind,
    severity: filters.severity,
    state: filters.state,
    agent: filters.agent,
    window: filters.window,
    limit: "25",
  });
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
