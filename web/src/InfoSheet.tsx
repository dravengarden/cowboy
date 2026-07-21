import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Box,
  Button,
  ButtonBase,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  IconButton,
  LinearProgress,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import { ExpandMore, Refresh } from "@mui/icons-material";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { useSkills } from "./store";
import {
  acceptedScheduleTime,
  type JsonRecord,
  num,
  type ProviderUsage,
  record,
  relativeUpdateTime,
  scheduledResetCountdown,
  type UsageLimit,
  nearestAvailableResetCredit,
  usageLimits,
  type UsageSnapshot,
} from "./usageLimits";

function formatBytes(n: number): string {
  if (n < 1024) return `${String(n)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(1)} ${units[i] ?? "B"}`;
}

function InfoRow({ k, v }: { k: string; v: string }): React.JSX.Element {
  return (
    <Stack direction="row" spacing={2} sx={{ justifyContent: "space-between", alignItems: "baseline" }}>
      <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0 }}>
        {k}
      </Typography>
      <Typography variant="body2" sx={{ wordBreak: "break-all", textAlign: "right" }}>
        {v}
      </Typography>
    </Stack>
  );
}

interface MetricsData {
  db_bytes: number;
  events_rows: number;
  sessions_live: number;
  sessions_deleted: number;
  daemon_rss_bytes: number;
}

function str(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function fullDateTime(epochSeconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(epochSeconds * 1000));
}

function resetText(epochSeconds: number | undefined): string | undefined {
  if (epochSeconds === undefined) return undefined;
  const date = new Date(epochSeconds * 1000);
  const delta = Math.max(0, date.getTime() - Date.now());
  const mins = Math.ceil(delta / 60_000);
  const relative = mins < 60
    ? `${String(mins)}m`
    : mins < 1440
    ? `${String(Math.floor(mins / 60))}h ${String(mins % 60)}m`
    : `${String(Math.floor(mins / 1440))}d ${String(Math.floor((mins % 1440) / 60))}h`;
  return `Resets in ${relative} · ${fullDateTime(epochSeconds)}`;
}

function LimitRow({ limit }: { limit: UsageLimit }): React.JSX.Element {
  return (
    <Stack spacing={0.65}>
      <Stack direction="row" justifyContent="space-between" spacing={1}>
        <Typography variant="body2">{limit.label}</Typography>
        <Typography variant="body2" sx={{ fontWeight: 600 }}>{limit.remaining}% remaining</Typography>
      </Stack>
      <LinearProgress
        variant="determinate"
        value={limit.remaining}
        sx={{ height: 7, borderRadius: 99, bgcolor: "action.selected", "& .MuiLinearProgress-bar": { borderRadius: 99 } }}
      />
      {resetText(limit.resetsAt) && (
        <Typography variant="caption" color="text.secondary">{resetText(limit.resetsAt)}</Typography>
      )}
    </Stack>
  );
}

function ProviderUsageCard({ usage, schedule, now, onUsageChanged }: {
  usage: ProviderUsage;
  schedule: { fire_at_ms: number } | undefined;
  now: number;
  onUsageChanged: () => Promise<void>;
}): React.JSX.Element {
  const [resetOpen, setResetOpen] = useState(false);
  const [resetMode, setResetMode] = useState<"schedule" | "now">("schedule");
  const [fireAt, setFireAt] = useState("");
  const [confirmText, setConfirmText] = useState("");
  const [resetBusy, setResetBusy] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  const limits = useMemo(() => usageLimits(usage), [usage]);
  const account = record(usage.account?.account);
  const plan = account ? str(account.planType) : undefined;
  const resetCredits = record(usage.rate_limits?.rateLimitResetCredits);
  const availableCredits = resetCredits ? num(resetCredits.availableCount) : undefined;
  const credits = Array.isArray(resetCredits?.credits)
    ? resetCredits.credits.map(record).filter((v): v is JsonRecord => v !== undefined)
    : [];
  const nearestCredit = nearestAvailableResetCredit(usage);
  const nearestCreditId = str(nearestCredit?.id);
  const summary = record(usage.activity?.summary);
  const sessionUsage = record(usage.activity?.session);
  const sessionCost = record(sessionUsage?.cost);
  const title = usage.provider === "claude-code" ? "Claude Code" : usage.provider === "gemini" ? "Gemini" : "Codex";
  const scheduleValid = fireAt !== "" && new Date(fireAt).getTime() > Date.now();
  const openResetDialog = () => {
    setResetMode("schedule");
    setFireAt("");
    setConfirmText("");
    setResetError(null);
    setResetOpen(true);
  };
  const closeResetDialog = () => {
    if (resetBusy) return;
    setResetOpen(false);
    setResetMode("schedule");
    setFireAt("");
    setConfirmText("");
    setResetError(null);
  };
  const cancelSchedule = async (): Promise<void> => {
    setResetBusy(true);
    setResetError(null);
    try {
      const response = await fetch("/api/usage/codex/reset/schedule", { method: "DELETE" });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${String(response.status)}`);
      await onUsageChanged();
    } catch (cause) {
      setResetError(cause instanceof Error ? cause.message : "Could not cancel schedule");
    } finally {
      setResetBusy(false);
    }
  };
  const submitReset = async (): Promise<void> => {
    if (resetBusy || confirmText !== "confirm" ||
      (resetMode === "schedule" && !scheduleValid)) return;
    setResetBusy(true);
    setResetError(null);
    try {
      const response = resetMode === "schedule"
        ? await fetch("/api/usage/codex/reset/schedule", {
          method: "PUT",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ fire_at_ms: new Date(fireAt).getTime(), confirm: confirmText }),
        })
        : await fetch("/api/usage/codex/reset", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ confirm: confirmText, expected_credit_id: nearestCreditId }),
        });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${String(response.status)}`);
      setResetOpen(false);
      setResetMode("schedule");
      setConfirmText("");
      await onUsageChanged();
    } catch (cause) {
      setResetError(cause instanceof Error ? cause.message : "Could not use reset");
    } finally {
      setResetBusy(false);
    }
  };
  useConfirmEnter(resetOpen, () => void submitReset());

  return (
    <Box sx={{ border: 1, borderColor: "divider", borderRadius: 2, px: 1.5, py: 1.4 }}>
      <Stack spacing={1.35}>
        <Stack direction="row" justifyContent="space-between" alignItems="baseline">
          <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>{title}</Typography>
          <Typography variant="caption" color={usage.status === "available" ? "success.main" : "text.secondary"}>
            {plan ? plan.toUpperCase() : usage.status === "available" ? "Live" : "Unavailable"}
          </Typography>
        </Stack>
        {limits.map((limit) => <LimitRow key={limit.id} limit={limit} />)}
        {usage.provider === "codex" && credits.length > 0 && (
          <Stack spacing={0} sx={{ pt: 0.5 }}>
            <Stack direction="row" justifyContent="space-between" alignItems="baseline" sx={{ pb: 0.75 }}>
              <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>Usage limit resets</Typography>
              {availableCredits !== undefined && (
                <Typography variant="caption" color="text.secondary">
                  {String(availableCredits)} available
                </Typography>
              )}
            </Stack>
            <Divider />
            {credits.map((credit, index) => {
              const expiresAt = num(credit.expiresAt);
              const actionable = str(credit.id) === nearestCreditId;
              const row = (
                <Box sx={{ py: 1.1, width: "100%", textAlign: "left" }}>
                  <Stack direction="row" justifyContent="space-between" spacing={1} alignItems="baseline">
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      {str(credit.title) ?? "Rate-limit reset"}
                    </Typography>
                    <Typography variant="caption" color={actionable ? "primary.main" : "text.secondary"} fontWeight={actionable ? 700 : 400}>
                      {actionable ? schedule ? "Scheduled" : "Use next" : "Available"}
                    </Typography>
                  </Stack>
                  <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.25 }}>
                    {expiresAt === undefined ? "No expiry reported" : `Expires ${fullDateTime(expiresAt)}`}
                  </Typography>
                </Box>
              );
              return (
                <Box
                  key={str(credit.id) ?? index}
                  sx={{ borderBottom: index < credits.length - 1 ? 1 : 0, borderColor: "divider" }}
                >
                  {actionable && !schedule
                    ? <ButtonBase onClick={openResetDialog} sx={{ width: "100%", borderRadius: 1 }}>{row}</ButtonBase>
                    : row}
                  {actionable && schedule && (
                    <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1} sx={{ pb: 1.1 }}>
                      <Box>
                        <Typography variant="caption" color="primary.main" fontWeight={700}>
                          {scheduledResetCountdown(schedule.fire_at_ms, now)}
                        </Typography>
                        <Typography variant="caption" color="text.secondary" sx={{ display: "block" }}>
                          {fullDateTime(schedule.fire_at_ms / 1000)}
                        </Typography>
                      </Box>
                      <Button size="small" disabled={resetBusy} onClick={() => void cancelSchedule()}>Cancel</Button>
                    </Stack>
                  )}
                </Box>
              );
            })}
            {resetError && !resetOpen && <Typography color="error.main" variant="caption">{resetError}</Typography>}
          </Stack>
        )}
        {summary && num(summary.lifetimeTokens) !== undefined && (
          <InfoRow k="Lifetime tokens" v={num(summary.lifetimeTokens)?.toLocaleString() ?? "—"} />
        )}
        {sessionUsage && num(sessionUsage.used) !== undefined && num(sessionUsage.size) !== undefined && (
          <InfoRow
            k="Latest context"
            v={`${num(sessionUsage.used)?.toLocaleString() ?? "0"} / ${num(sessionUsage.size)?.toLocaleString() ?? "0"}`}
          />
        )}
        {sessionCost && num(sessionCost.amount) !== undefined && (
          <InfoRow
            k="Session cost"
            v={`${str(sessionCost.currency) ?? "USD"} ${String(num(sessionCost.amount) ?? 0)}`}
          />
        )}
        {limits.length === 0 && (
          <Typography variant="body2" color="text.secondary">{usage.error ?? "Detailed limits are not available."}</Typography>
        )}
        <Typography variant="caption" color="text.secondary">
          {usage.source} · Updated {relativeUpdateTime(usage.observed_at_ms)}
        </Typography>
      </Stack>
      <Dialog open={resetOpen} onClose={closeResetDialog} fullWidth maxWidth="xs">
        <DialogTitle>{resetMode === "schedule" ? "Schedule nearest reset" : "Use nearest reset now?"}</DialogTitle>
        <DialogContent>
          <DialogContentText>
            {resetMode === "schedule"
              ? "At the selected time, Cowboy will use the earliest-expiring reset then available."
              : "Cowboy will immediately use the earliest-expiring available reset. This cannot be undone."}
          </DialogContentText>
          <ToggleButtonGroup
            exclusive
            fullWidth
            value={resetMode}
            onChange={(_event, value: "schedule" | "now" | null) => {
              if (!value || value === resetMode || resetBusy) return;
              setResetMode(value);
              setConfirmText("");
              setResetError(null);
            }}
            aria-label="Reset timing"
            sx={{ mt: 2 }}
          >
            <ToggleButton value="schedule">Schedule</ToggleButton>
            <ToggleButton value="now" color="error">Now</ToggleButton>
          </ToggleButtonGroup>
          {resetError && <Typography color="error.main" variant="body2" sx={{ mt: 1 }}>{resetError}</Typography>}
          {resetMode === "schedule" && (
            <TextField
              type="datetime-local"
              fullWidth
              label="Run at"
              value={fireAt}
              onChange={(event) => setFireAt(acceptedScheduleTime(event.target.value))}
              slotProps={{ inputLabel: { shrink: true } }}
              helperText="Choose a time at least one minute ahead"
              sx={{ mt: 2 }}
            />
          )}
          <TextField
            autoComplete="off"
            fullWidth
            label="Type confirm to continue"
            value={confirmText}
            onChange={(event) => setConfirmText(event.target.value)}
            sx={{ mt: 2 }}
          />
        </DialogContent>
        <DialogActions sx={{ px: 3, pb: 2 }}>
          <Button onClick={closeResetDialog} disabled={resetBusy}>Cancel</Button>
          <Button
            variant="contained"
            color={resetMode === "now" ? "error" : "primary"}
            disabled={resetBusy || confirmText !== "confirm" || (resetMode === "schedule" && !scheduleValid)}
            onClick={() => void submitReset()}
          >
            {resetBusy ? "Working…" : resetMode === "schedule" ? "Schedule reset" : "Reset now"}
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}

function UsageInfoSection(): React.JSX.Element {
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  const load = useCallback(async (manual: boolean): Promise<void> => {
    if (refreshing) return;
    setRefreshing(true);
    setError(null);
    try {
      const response = await fetch("/api/usage", { method: manual ? "POST" : "GET" });
      if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
      setSnapshot(await response.json() as UsageSnapshot);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Refresh failed");
    } finally {
      setRefreshing(false);
    }
  }, [refreshing]);
  useEffect(() => { void load(false); }, []); // load only when Info mounts
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => window.clearInterval(timer);
  }, []);
  const refreshed = relativeUpdateTime(snapshot?.refreshed_at_ms ?? 0, clock);
  const nextRefreshMinutes = snapshot?.next_refresh_at_ms
    ? Math.max(0, Math.ceil((snapshot.next_refresh_at_ms - clock) / 60_000))
    : null;

  return (
    <Stack spacing={1.25}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Box>
          <Typography variant="overline" color="text.secondary">Usage</Typography>
          <Typography variant="caption" color={error ? "error.main" : "text.secondary"} sx={{ display: "block" }}>
            {error ? `Refresh failed · Updated ${refreshed}` : `Updated ${refreshed}`}
            {nextRefreshMinutes !== null ? ` · Auto refresh in ${String(nextRefreshMinutes)}m` : ""}
          </Typography>
        </Box>
        <IconButton aria-label="Refresh usage" disabled={refreshing} onClick={() => void load(true)} sx={{ width: 44, height: 44 }}>
          {refreshing ? <CircularProgress size={20} /> : <Refresh />}
        </IconButton>
      </Stack>
      {snapshot?.providers.map((provider) => (
        <ProviderUsageCard
          key={provider.provider}
          usage={provider}
          schedule={snapshot.codex_reset_schedule}
          now={clock}
          onUsageChanged={() => load(false)}
        />
      ))}
      {!snapshot && !error && <Typography variant="body2" color="text.secondary">Loading usage…</Typography>}
    </Stack>
  );
}

// Storage/runtime metrics (GET /api/metrics). Migrated here from user Settings —
// it's daemon system info, not a user preference.
function StorageInfoSection(): React.JSX.Element {
  const [m, setM] = useState<MetricsData | null>(null);
  useEffect(() => {
    const ctrl = new AbortController();
    void fetch("/api/metrics", { signal: ctrl.signal })
      .then((r) => r.json() as Promise<MetricsData>)
      .then(setM)
      .catch(() => {
        /* leave as Loading… */
      });
    return () => {
      ctrl.abort();
    };
  }, []);
  if (!m) {
    return (
      <Typography variant="body2" sx={{ color: "text.secondary" }}>
        Loading…
      </Typography>
    );
  }
  return (
    <Stack spacing={1}>
      <InfoRow k="Database" v={formatBytes(m.db_bytes)} />
      <InfoRow k="Event rows" v={m.events_rows.toLocaleString()} />
      <InfoRow k="Live sessions" v={String(m.sessions_live)} />
      <InfoRow k="Deleted (purge ≤3d)" v={String(m.sessions_deleted)} />
      <InfoRow k="Daemon memory" v={formatBytes(m.daemon_rss_bytes)} />
    </Stack>
  );
}

// The Info tab's body — rendered inside the merged Settings sheet (no own Sheet
// wrapper). Holds the classifier/skills viewer and daemon system info.
export function InfoContent(): React.JSX.Element {
  const skills = useSkills();

  return (
    <Stack spacing={2.5} sx={{ mt: 1 }}>
        <UsageInfoSection />
        <Divider />
        <Box>
          <Typography variant="overline" color="text.secondary">
            Turn classifier
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
            Normal turn endings use isolated Codex Luna threads on one shared
            app-server; deterministic stop reasons need no model call.
          </Typography>
          <InfoRow k="Runtime" v="Codex app-server" />
          <InfoRow k="Model" v="gpt-5.6-luna" />
        </Box>

        {/* Skills — provider-agnostic capability units run at turn-end. Each is
            expandable to show the exact prompt + how the output is extracted, so
            the judgment logic is inspectable (not a black box). */}
        <Box>
          <Typography variant="overline" color="text.secondary">
            Skills
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
            Run after each turn to classify what the agent did.
          </Typography>
          <Stack spacing={1}>
            {skills.length === 0 && (
              <Typography variant="caption" color="text.secondary">
                No skills reported (connecting…).
              </Typography>
            )}
            {skills.map((sk) => (
              <Accordion key={sk.id} disableGutters sx={{ borderRadius: 2, "&:before": { display: "none" } }}>
                <AccordionSummary expandIcon={<ExpandMore />}>
                  <Box>
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      {sk.title}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      {sk.description}
                    </Typography>
                  </Box>
                </AccordionSummary>
                <AccordionDetails>
                  <Stack spacing={1.5}>
                    <Box>
                      <Typography variant="caption" color="text.secondary" sx={{ fontWeight: 600 }}>
                        Prompt
                      </Typography>
                      <Box
                        component="pre"
                        sx={{
                          m: 0,
                          mt: 0.5,
                          p: 1,
                          borderRadius: 1.5,
                          bgcolor: "action.hover",
                          fontSize: 11,
                          lineHeight: 1.5,
                          whiteSpace: "pre-wrap",
                          wordBreak: "break-word",
                          maxHeight: 260,
                          overflow: "auto",
                        }}
                      >
                        {sk.prompt_template}
                      </Box>
                    </Box>
                    <Box>
                      <Typography variant="caption" color="text.secondary" sx={{ fontWeight: 600 }}>
                        Extraction
                      </Typography>
                      <Typography variant="body2" sx={{ mt: 0.5 }}>
                        {sk.extract}
                      </Typography>
                    </Box>
                  </Stack>
                </AccordionDetails>
              </Accordion>
            ))}
          </Stack>
        </Box>

        <Divider />
        <Stack spacing={1}>
          <Typography variant="overline" color="text.secondary">
            Storage
          </Typography>
          <StorageInfoSection />
        </Stack>

        <Divider />
        <Stack spacing={0.5}>
          <Typography variant="overline" color="text.secondary">
            About
          </Typography>
          <Typography variant="body2" color="text.secondary">
            cowboy v0.1 — multi-agent panel driving Claude Code / Codex over ACP.
          </Typography>
        </Stack>
      </Stack>
  );
}
