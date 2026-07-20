import { useCallback, useEffect, useState } from "react";
import { alpha, Box, Button, CircularProgress, Divider, Stack, Typography } from "@mui/material";
import { Refresh } from "@mui/icons-material";

export interface ProviderActionLog {
  id: number;
  provider: string;
  action: string;
  trigger: "manual" | "scheduled";
  status: "scheduled" | "started" | "retrying" | "succeeded" | "failed" | "unknown" | "cancelled";
  phase: string;
  message: string;
  credit_id?: string;
  idempotency_suffix?: string;
  created_at_ms: number;
}

const STATUS_COLOR: Record<ProviderActionLog["status"], string> = {
  scheduled: "primary.main", started: "primary.main", retrying: "warning.main", succeeded: "success.main",
  failed: "error.main", unknown: "warning.main", cancelled: "text.disabled",
};

function time(value: number): string {
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(value));
}

export function UsageLogs({ dense = false }: { dense?: boolean }): React.JSX.Element {
  const [logs, setLogs] = useState<ProviderActionLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async (): Promise<void> => {
    setLoading(true);
    try {
      const response = await fetch("/api/usage/logs");
      if (!response.ok) throw new Error(await response.text() || `HTTP ${String(response.status)}`);
      setLogs(await response.json() as ProviderActionLog[]);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not load logs");
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void load();
    const timer = globalThis.setInterval(() => void load(), 15_000);
    return () => globalThis.clearInterval(timer);
  }, [load]);
  return (
    <Stack spacing={1.25} sx={{ mt: dense ? 0 : 1 }}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Box>
          <Typography variant={dense ? "subtitle2" : "overline"} fontWeight={750}>Automation logs</Typography>
          <Typography variant="caption" color="text.secondary" display="block">Persistent reset scheduling and execution audit</Typography>
        </Box>
        <Button size="small" onClick={() => void load()} disabled={loading} startIcon={loading ? <CircularProgress size={13} /> : <Refresh fontSize="small" />}>Refresh</Button>
      </Stack>
      <Divider />
      {error && <Typography variant="caption" color="error.main">{error}</Typography>}
      {!loading && logs.length === 0 && <Typography variant="body2" color="text.secondary">No automation activity recorded yet.</Typography>}
      <Stack spacing={0.75}>
        {logs.map((entry) => (
          <Box key={entry.id} sx={{ display: "grid", gridTemplateColumns: "10px minmax(0, 1fr)", gap: 1, p: dense ? 0.8 : 1.1, borderRadius: 1.5, bgcolor: (theme) => alpha(theme.palette.text.primary, 0.035) }}>
            <Box sx={{ width: 8, height: 8, mt: 0.65, borderRadius: "50%", bgcolor: STATUS_COLOR[entry.status] }} />
            <Box sx={{ minWidth: 0 }}>
              <Stack direction="row" justifyContent="space-between" spacing={1}>
                <Typography variant="body2" fontWeight={700}>{entry.status === "unknown" ? "Outcome unknown — no retry" : entry.status}</Typography>
                <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0 }}>{time(entry.created_at_ms)}</Typography>
              </Stack>
              <Typography variant="caption" color="text.secondary">{entry.trigger} · {entry.phase}{entry.idempotency_suffix ? ` · key …${entry.idempotency_suffix}` : ""}</Typography>
              <Typography variant="body2" sx={{ mt: 0.35, overflowWrap: "anywhere" }}>{entry.message}</Typography>
              {entry.credit_id && <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.25 }}>Credit …{entry.credit_id.slice(-10)}</Typography>}
            </Box>
          </Box>
        ))}
      </Stack>
    </Stack>
  );
}
