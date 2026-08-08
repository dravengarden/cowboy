import { useCallback, useEffect, useState } from "react";
import {
  alpha,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Divider,
  IconButton,
  MenuItem,
  Stack,
  TextField,
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
  type DiagnosticLogDetail,
  type DiagnosticLogFilters,
  type DiagnosticLogPage,
  type DiagnosticLogSummary,
  diagnosticKindLabel,
  diagnosticLogUrl,
} from "./diagnosticLogs";
import { NetworkButton } from "./NetworkActionFeedback";

const SEVERITY_COLOR: Record<DiagnosticLogSummary["severity"], string> = {
  info: "primary.main",
  warning: "warning.main",
  error: "error.main",
  critical: "error.dark",
};

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
  const [filters, setFilters] = useState<DiagnosticLogFilters>(DEFAULT_DIAGNOSTIC_LOG_FILTERS);
  const [logs, setLogs] = useState<DiagnosticLogSummary[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [details, setDetails] = useState<Record<string, { loading: boolean; value?: DiagnosticLogDetail; error?: string }>>({});
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const loadPage = useCallback(async (
    cursor: string | undefined,
    append: boolean,
    signal?: AbortSignal,
  ): Promise<void> => {
    if (append) setLoadingMore(true);
    else setLoading(true);
    try {
      const response = await fetch(
        diagnosticLogUrl(filters, cursor),
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

  const updateFilter = <K extends keyof DiagnosticLogFilters>(
    key: K,
    value: DiagnosticLogFilters[K],
  ): void => setFilters((current) => ({ ...current, [key]: value }));

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
            Session failures, provider errors, cache disruptions, and automation audit
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
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: dense ? "repeat(2, minmax(0, 1fr))" : { xs: "repeat(2, minmax(0, 1fr))", sm: "repeat(5, minmax(0, 1fr))" },
          gap: 0.75,
        }}
      >
        <TextField select size="small" label="Type" value={filters.kind} onChange={(event) => updateFilter("kind", event.target.value as DiagnosticLogFilters["kind"])}>
          <MenuItem value="all">All</MenuItem>
          <MenuItem value="session_error">Session</MenuItem>
          <MenuItem value="provider_error">Provider</MenuItem>
          <MenuItem value="cache_anomaly">Cache</MenuItem>
          <MenuItem value="automation">Automation</MenuItem>
        </TextField>
        <TextField select size="small" label="Severity" value={filters.severity} onChange={(event) => updateFilter("severity", event.target.value as DiagnosticLogFilters["severity"])}>
          <MenuItem value="all">All</MenuItem>
          <MenuItem value="error">Error</MenuItem>
          <MenuItem value="critical">Critical</MenuItem>
          <MenuItem value="warning">Warning</MenuItem>
          <MenuItem value="info">Info</MenuItem>
        </TextField>
        <TextField select size="small" label="State" value={filters.state} onChange={(event) => updateFilter("state", event.target.value as DiagnosticLogFilters["state"])}>
          <MenuItem value="all">All</MenuItem>
          <MenuItem value="active">Active</MenuItem>
          <MenuItem value="recovered">Recovered</MenuItem>
          <MenuItem value="failed">Failed</MenuItem>
          <MenuItem value="succeeded">Succeeded</MenuItem>
          <MenuItem value="observed">Observed</MenuItem>
          <MenuItem value="scheduled">Scheduled</MenuItem>
          <MenuItem value="started">Started</MenuItem>
          <MenuItem value="retrying">Retrying</MenuItem>
          <MenuItem value="unknown">Unknown</MenuItem>
          <MenuItem value="cancelled">Cancelled</MenuItem>
        </TextField>
        <TextField select size="small" label="Runtime" value={filters.agent} onChange={(event) => updateFilter("agent", event.target.value as DiagnosticLogFilters["agent"])}>
          <MenuItem value="all">All</MenuItem>
          <MenuItem value="codex">Codex</MenuItem>
          <MenuItem value="claude">Claude Code</MenuItem>
        </TextField>
        <TextField select size="small" label="Window" value={filters.window} onChange={(event) => updateFilter("window", event.target.value as DiagnosticLogFilters["window"])}>
          <MenuItem value="1h">1 hour</MenuItem>
          <MenuItem value="24h">24 hours</MenuItem>
          <MenuItem value="7d">7 days</MenuItem>
          <MenuItem value="30d">30 days</MenuItem>
        </TextField>
      </Box>
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
