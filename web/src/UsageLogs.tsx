import { useCallback, useEffect, useRef, useState } from "react";
import {
  alpha,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Divider,
  IconButton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  Check,
  ContentCopy,
  ExpandLess,
  ExpandMore,
  Refresh,
} from "@mui/icons-material";
import { copyText } from "./clipboard";
import {
  DEFAULT_DIAGNOSTIC_LOG_FILTERS,
  cloneDiagnosticLogFilters,
  loadDiagnosticLogFilters,
  persistDiagnosticLogFilters,
  type DiagnosticLogAgent,
  type DiagnosticLogDetail,
  type DiagnosticLogFilters,
  type DiagnosticLogKind,
  type DiagnosticLogPage,
  type DiagnosticLogSeverity,
  type DiagnosticLogState,
  type DiagnosticLogSummary,
  diagnosticKindLabel,
  diagnosticLogUrl,
} from "./diagnosticLogs";
import { NetworkButton } from "./NetworkActionFeedback";
import {
  ActiveFilterChips,
  type FilterChipOption,
  FilterButton,
  MultiSelectChipGroup,
  TimeRangeButton,
} from "./ObservabilityFilters";
import { Sheet } from "./Sheet";

const SEVERITY_COLOR: Record<DiagnosticLogSummary["severity"], string> = {
  info: "primary.main",
  warning: "warning.main",
  error: "error.main",
  critical: "error.dark",
};

const KIND_OPTIONS: readonly FilterChipOption<DiagnosticLogKind>[] = [
  { value: "session_error", label: "Session", color: "error" },
  { value: "provider_error", label: "Provider", color: "warning" },
  { value: "cache_anomaly", label: "Cache", color: "secondary" },
  { value: "automation", label: "Automation", color: "info" },
];
const SEVERITY_OPTIONS: readonly FilterChipOption<DiagnosticLogSeverity>[] = [
  { value: "critical", label: "Critical", color: "error" },
  { value: "error", label: "Error", color: "error" },
  { value: "warning", label: "Warning", color: "warning" },
  { value: "info", label: "Info", color: "info" },
];
const STATE_OPTIONS: readonly FilterChipOption<DiagnosticLogState>[] = [
  { value: "active", label: "Active", color: "error" },
  { value: "failed", label: "Failed", color: "error" },
  { value: "recovered", label: "Recovered", color: "success" },
  { value: "succeeded", label: "Succeeded", color: "success" },
  { value: "observed", label: "Observed", color: "secondary" },
  { value: "retrying", label: "Retrying", color: "warning" },
  { value: "scheduled", label: "Scheduled", color: "info" },
  { value: "started", label: "Started", color: "info" },
  { value: "unknown", label: "Unknown", color: "warning" },
  { value: "cancelled", label: "Cancelled" },
];
const AGENT_OPTIONS: readonly FilterChipOption<DiagnosticLogAgent>[] = [
  { value: "claude", label: "Claude Code", color: "secondary" },
  { value: "codex", label: "Codex", color: "info" },
];

function optionLabel<T extends string>(options: readonly FilterChipOption<T>[], value: T): string {
  return options.find((option) => option.value === value)?.label ?? value;
}

function time(value: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}

function shortRef(value: string): string {
  return value.length <= 20 ? value : `${value.slice(0, 9)}…${value.slice(-8)}`;
}

function detailEvidence(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function LogDetail({
  state,
  copiedKey,
  onCopy,
}: {
  state: { loading: boolean; value?: DiagnosticLogDetail; error?: string };
  copiedKey: string | null;
  onCopy: (key: string, value: string) => void;
}): React.JSX.Element {
  if (state.loading) {
    return (
      <Stack direction="row" spacing={1} alignItems="center" sx={{ py: 1 }}>
        <CircularProgress size={15} />
        <Typography variant="caption" color="text.secondary">Loading detail…</Typography>
      </Stack>
    );
  }
  if (state.error || !state.value) {
    return <Typography variant="caption" color="error.main">{state.error ?? "Detail unavailable"}</Typography>;
  }
  return (
    <Stack spacing={1} sx={{ pt: 1 }}>
      {state.value.sections.map((section) => (
        <Box key={section.title}>
          <Typography variant="caption" color="text.secondary" fontWeight={700}>
            {section.title}
          </Typography>
          <Stack spacing={0.35} sx={{ mt: 0.4 }}>
            {section.fields.map((field) => {
              const copyKey = `${state.value?.id ?? "detail"}:${section.title}:${field.label}`;
              return (
                <Stack
                  key={`${section.title}:${field.label}`}
                  direction="row"
                  spacing={1}
                  alignItems="flex-start"
                  justifyContent="space-between"
                >
                  <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0 }}>
                    {field.label}
                  </Typography>
                  <Stack direction="row" spacing={0.25} alignItems="flex-start" sx={{ minWidth: 0 }}>
                    <Typography
                      variant="caption"
                      sx={{ textAlign: "right", overflowWrap: "anywhere", fontFamily: field.copyable ? "monospace" : undefined }}
                    >
                      {field.value}
                    </Typography>
                    {field.copyable && (
                      <Tooltip title="Copy value">
                        <IconButton
                          size="small"
                          aria-label={`Copy ${field.label}`}
                          onClick={() => onCopy(copyKey, field.value)}
                          sx={{ p: 0.25 }}
                        >
                          {copiedKey === copyKey ? <Check sx={{ fontSize: 14 }} /> : <ContentCopy sx={{ fontSize: 13 }} />}
                        </IconButton>
                      </Tooltip>
                    )}
                  </Stack>
                </Stack>
              );
            })}
          </Stack>
        </Box>
      ))}
      {state.value.evidence !== undefined && (
        <Box component="details">
          <Typography component="summary" variant="caption" color="text.secondary" sx={{ cursor: "pointer" }}>
            Structured evidence
          </Typography>
          <Box
            component="pre"
            sx={{ m: 0, mt: 0.6, p: 0.8, maxHeight: 280, overflow: "auto", borderRadius: 1, bgcolor: "action.hover", fontSize: "0.68rem", whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}
          >
            {detailEvidence(state.value.evidence)}
          </Box>
        </Box>
      )}
    </Stack>
  );
}

export function UsageLogs({ dense = false }: { dense?: boolean }): React.JSX.Element {
  const [filters, setFilters] = useState<DiagnosticLogFilters>(() => loadDiagnosticLogFilters());
  const [draftFilters, setDraftFilters] = useState<DiagnosticLogFilters>(() => loadDiagnosticLogFilters());
  const [filterOpen, setFilterOpen] = useState(false);
  const [logs, setLogs] = useState<DiagnosticLogSummary[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [details, setDetails] = useState<Record<string, { loading: boolean; value?: DiagnosticLogDetail; error?: string }>>({});
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const rangeAnchorMs = useRef(Date.now());

  useEffect(() => {
    persistDiagnosticLogFilters(filters);
  }, [filters]);

  const loadPage = useCallback(async (
    cursor: string | undefined,
    append: boolean,
    signal?: AbortSignal,
  ): Promise<void> => {
    if (append) setLoadingMore(true);
    else setLoading(true);
    try {
      if (!append) rangeAnchorMs.current = Date.now();
      const response = await fetch(
        diagnosticLogUrl(filters, cursor, rangeAnchorMs.current),
        signal ? { signal } : {},
      );
      if (!response.ok) throw new Error(await response.text() || `HTTP ${String(response.status)}`);
      const page = await response.json() as DiagnosticLogPage;
      setLogs((current) => {
        if (!append) return page.items;
        const merged = new Map(current.map((item) => [item.id, item]));
        for (const item of page.items) merged.set(item.id, item);
        return [...merged.values()];
      });
      setNextCursor(page.next_cursor ?? null);
      setError(null);
    } catch (cause) {
      if (cause instanceof DOMException && cause.name === "AbortError") return;
      setError(cause instanceof Error ? cause.message : "Could not load diagnostic logs");
    } finally {
      if (append) setLoadingMore(false);
      else setLoading(false);
    }
  }, [filters]);

  const refresh = useCallback(() => loadPage(undefined, false), [loadPage]);

  useEffect(() => {
    const controller = new AbortController();
    setExpandedId(null);
    void loadPage(undefined, false, controller.signal);
    const timer = globalThis.setInterval(() => void loadPage(undefined, false), 30_000);
    return () => {
      controller.abort();
      globalThis.clearInterval(timer);
    };
  }, [loadPage]);

  const openFilters = (): void => {
    setDraftFilters(cloneDiagnosticLogFilters(filters));
    setFilterOpen(true);
  };
  const resetFilters = (): void => {
    const next = cloneDiagnosticLogFilters(DEFAULT_DIAGNOSTIC_LOG_FILTERS);
    setFilters(next);
    setDraftFilters(cloneDiagnosticLogFilters(next));
    setFilterOpen(false);
  };
  const activeFilterCount = filters.kinds.length + filters.severities.length +
    filters.states.length + filters.agents.length;

  const copy = (key: string, value: string): void => {
    void copyText(value).then((copied) => {
      if (!copied) return;
      setCopiedKey(key);
      globalThis.setTimeout(() => setCopiedKey((current) => current === key ? null : current), 1_500);
    });
  };

  const toggleDetail = (entry: DiagnosticLogSummary): void => {
    if (expandedId === entry.id) {
      setExpandedId(null);
      return;
    }
    setExpandedId(entry.id);
    setDetails((current) => ({ ...current, [entry.id]: { loading: true } }));
    void fetch(`/api/logs/${encodeURIComponent(entry.id)}`)
      .then(async (response) => {
        if (!response.ok) throw new Error(await response.text() || `HTTP ${String(response.status)}`);
        return response.json() as Promise<DiagnosticLogDetail>;
      })
      .then((value) => setDetails((current) => ({ ...current, [entry.id]: { loading: false, value } })))
      .catch((cause) => setDetails((current) => ({
        ...current,
        [entry.id]: {
          loading: false,
          error: cause instanceof Error ? cause.message : "Could not load detail",
        },
      })));
  };

  return (
    <Stack spacing={1.25} sx={{ mt: dense ? 0 : 1 }}>
      <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
        <Box>
          <Typography variant={dense ? "subtitle2" : "overline"} fontWeight={750}>Diagnostic logs</Typography>
          <Typography variant="caption" color="text.secondary" display="block">
            Serious failures by default; include retryable and audit events from Filters
          </Typography>
        </Box>
        <NetworkButton
          size="small"
          disabled={loading}
          startIcon={<Refresh fontSize="small" />}
          networkAction={refresh}
        >
          Refresh
        </NetworkButton>
      </Stack>
      <Stack spacing={0.75}>
        <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
          <TimeRangeButton
            value={filters.timeRange}
            onChange={(timeRange) => setFilters((current) => ({ ...current, timeRange }))}
            defaultValue={DEFAULT_DIAGNOSTIC_LOG_FILTERS.timeRange}
            maxDurationMs={365 * 86_400_000}
          />
          <FilterButton count={activeFilterCount} onClick={openFilters} />
        </Stack>
        <ActiveFilterChips
          items={[
            ...filters.kinds.map((value) => ({
              key: `kind:${value}`,
              label: optionLabel(KIND_OPTIONS, value),
              color: KIND_OPTIONS.find((option) => option.value === value)?.color,
              onDelete: () => setFilters((current) => ({ ...current, kinds: current.kinds.filter((item) => item !== value) })),
            })),
            ...filters.severities.map((value) => ({
              key: `severity:${value}`,
              label: optionLabel(SEVERITY_OPTIONS, value),
              color: SEVERITY_OPTIONS.find((option) => option.value === value)?.color,
              onDelete: () => setFilters((current) => ({ ...current, severities: current.severities.filter((item) => item !== value) })),
            })),
            ...filters.states.map((value) => ({
              key: `state:${value}`,
              label: optionLabel(STATE_OPTIONS, value),
              color: STATE_OPTIONS.find((option) => option.value === value)?.color,
              onDelete: () => setFilters((current) => ({ ...current, states: current.states.filter((item) => item !== value) })),
            })),
            ...filters.agents.map((value) => ({
              key: `agent:${value}`,
              label: optionLabel(AGENT_OPTIONS, value),
              color: AGENT_OPTIONS.find((option) => option.value === value)?.color,
              onDelete: () => setFilters((current) => ({ ...current, agents: current.agents.filter((item) => item !== value) })),
            })),
          ]}
        />
      </Stack>
      <Sheet
        open={filterOpen}
        onClose={() => setFilterOpen(false)}
        title="Filter diagnostic logs"
        desktopMaxWidth={560}
        mobileDismiss="none"
        floatingActions={false}
      >
        <Stack spacing={2} sx={{ pt: 0.5, pb: 1 }}>
          <MultiSelectChipGroup label="Type" options={KIND_OPTIONS} value={draftFilters.kinds} onChange={(kinds) => setDraftFilters((current) => ({ ...current, kinds }))} />
          <MultiSelectChipGroup label="Severity" options={SEVERITY_OPTIONS} value={draftFilters.severities} onChange={(severities) => setDraftFilters((current) => ({ ...current, severities }))} />
          <Typography variant="caption" color="text.secondary" sx={{ mt: -1.25 }}>
            Critical and Error are blocking or session-ending. Warning includes retryable provider attempts and cache disruption.
          </Typography>
          <MultiSelectChipGroup label="State" options={STATE_OPTIONS} value={draftFilters.states} onChange={(states) => setDraftFilters((current) => ({ ...current, states }))} />
          <MultiSelectChipGroup label="Runtime" options={AGENT_OPTIONS} value={draftFilters.agents} onChange={(agents) => setDraftFilters((current) => ({ ...current, agents }))} />
          <Stack direction="row" spacing={1} justifyContent="space-between">
            <Stack direction="row" spacing={0.5}>
              <Button onClick={() => setDraftFilters((current) => ({ ...current, kinds: [], severities: [], states: [], agents: [] }))}>Clear selections</Button>
              <Button onClick={resetFilters}>Reset</Button>
            </Stack>
            <Stack direction="row" spacing={1}>
              <Button onClick={() => setFilterOpen(false)}>Cancel</Button>
              <Button variant="contained" onClick={() => { setFilters(draftFilters); setFilterOpen(false); }}>Apply</Button>
            </Stack>
          </Stack>
        </Stack>
      </Sheet>
      <Divider />
      {error && <Typography variant="caption" color="error.main">{error}</Typography>}
      {!loading && logs.length === 0 && (
        <Typography variant="body2" color="text.secondary">No diagnostic activity matches these filters.</Typography>
      )}
      <Stack spacing={0.75}>
        {logs.map((entry) => {
          const expanded = expandedId === entry.id;
          const detailState = details[entry.id];
          const meta = [
            diagnosticKindLabel(entry.kind),
            entry.agent === "claude" ? "Claude Code" : entry.agent === "codex" ? "Codex" : undefined,
            entry.model,
            entry.session_ref ? `session ${shortRef(entry.session_ref)}` : undefined,
          ].filter((value): value is string => value !== undefined);
          return (
            <Box
              key={entry.id}
              sx={{
                display: "grid",
                gridTemplateColumns: "10px minmax(0, 1fr)",
                gap: 1,
                p: dense ? 0.8 : 1.1,
                borderRadius: 1.5,
                bgcolor: (theme) => alpha(theme.palette.text.primary, 0.035),
              }}
            >
              <Box sx={{ width: 8, height: 8, mt: 0.65, borderRadius: "50%", bgcolor: SEVERITY_COLOR[entry.severity] }} />
              <Box sx={{ minWidth: 0 }}>
                <Stack direction="row" spacing={0.5} alignItems="flex-start">
                  <ButtonBase
                    onClick={() => toggleDetail(entry)}
                    aria-expanded={expanded}
                    sx={{ display: "block", flex: 1, minWidth: 0, textAlign: "left", borderRadius: 1 }}
                  >
                    <Stack direction="row" justifyContent="space-between" spacing={1}>
                      <Stack direction="row" spacing={0.6} alignItems="center" sx={{ minWidth: 0 }}>
                        <Typography variant="body2" fontWeight={700} noWrap>{entry.title}</Typography>
                        <Chip label={entry.state} size="small" sx={{ height: 19, fontSize: "0.62rem" }} />
                      </Stack>
                      <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0 }}>{time(entry.occurred_at_ms)}</Typography>
                    </Stack>
                    <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.15, overflowWrap: "anywhere" }}>
                      {meta.join(" · ")}
                    </Typography>
                    <Typography variant="body2" sx={{ mt: 0.35, overflowWrap: "anywhere" }}>{entry.summary}</Typography>
                  </ButtonBase>
                  <Tooltip title={copiedKey === entry.id ? "Copied" : "Copy log ID"}>
                    <IconButton size="small" aria-label="Copy log ID" onClick={() => copy(entry.id, entry.id)} sx={{ mt: -0.35 }}>
                      {copiedKey === entry.id ? <Check sx={{ fontSize: 16 }} /> : <ContentCopy sx={{ fontSize: 15 }} />}
                    </IconButton>
                  </Tooltip>
                  <IconButton size="small" aria-label={expanded ? "Collapse log detail" : "Expand log detail"} onClick={() => toggleDetail(entry)} sx={{ mt: -0.35 }}>
                    {expanded ? <ExpandLess sx={{ fontSize: 18 }} /> : <ExpandMore sx={{ fontSize: 18 }} />}
                  </IconButton>
                </Stack>
                {expanded && detailState && (
                  <LogDetail state={detailState} copiedKey={copiedKey} onCopy={copy} />
                )}
              </Box>
            </Box>
          );
        })}
      </Stack>
      {nextCursor && (
        <Button
          size="small"
          disabled={loadingMore}
          onClick={() => void loadPage(nextCursor, true)}
          startIcon={loadingMore ? <CircularProgress size={14} /> : undefined}
        >
          Load more
        </Button>
      )}
      <Typography variant="caption" color="text.disabled">
        Lists and details load on demand. PostgreSQL keeps the durable index; raw service evidence remains in VictoriaLogs.
      </Typography>
    </Stack>
  );
}
