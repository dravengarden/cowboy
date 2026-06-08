import { useEffect, useState } from "react";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Box,
  Button,
  Chip,
  CircularProgress,
  Divider,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { CheckCircle, ExpandMore } from "@mui/icons-material";
import { Sheet } from "./Sheet";
import {
  runInferenceProbe,
  setInferenceConfig,
  setInferenceSecret,
  useInferenceConfig,
  useLastProbe,
  useSkills,
} from "./store";

// Static model list for now — Step 18 makes this a data-driven `ModelSource`
// (optionally fetched from the provider's /models). Ids are data, not control flow.
const DEEPSEEK_MODELS = [
  { id: "deepseek-v4-flash", label: "V4 Flash — fast & cheap (default)" },
  { id: "deepseek-v4-pro", label: "V4 Pro — thinking" },
];

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

// The Info sheet (opened from the info button left of the Settings gear). Holds
// the inference-provider config, the skills viewer, and the daemon system-info
// block (migrated out of user Settings).
export function InfoSheet({ open, onClose }: { open: boolean; onClose: () => void }): React.JSX.Element {
  const configs = useInferenceConfig();
  const ds = configs.find((c) => c.provider === "deepseek");
  const model = ds?.model || "deepseek-v4-flash";
  const keySet = ds?.key_set ?? false;
  const [keyInput, setKeyInput] = useState("");
  const probe = useLastProbe();
  const skills = useSkills();
  // Probe is fire-and-forget — the result lands later via the broadcast. Show a
  // spinner in between. The store builds a fresh `probe` object per result, so a
  // new reference (even with identical values) clears the pending flag.
  const [testing, setTesting] = useState(false);
  useEffect(() => {
    setTesting(false);
  }, [probe]);

  const saveKey = (): void => {
    const k = keyInput.trim();
    if (!k) return;
    setInferenceSecret("deepseek", k);
    setKeyInput(""); // never keep the key in component state
  };

  return (
    <Sheet open={open} onClose={onClose} title="Info">
      <Stack spacing={2.5} sx={{ mt: 1 }}>
        <Box>
          <Typography variant="overline" color="text.secondary">
            Inference provider
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
            Judges whether the agent is waiting for your reply (confirm-detection). The key is
            stored on the daemon and never shown again.
          </Typography>

          <Stack spacing={1.5}>
            <Stack direction="row" alignItems="center" justifyContent="space-between">
              <Typography variant="body2" sx={{ fontWeight: 600 }}>
                DeepSeek
              </Typography>
              {keySet
                ? <Chip size="small" color="success" icon={<CheckCircle />} label="Key set" />
                : <Chip size="small" color="warning" label="No key" />}
            </Stack>

            <Stack direction="row" spacing={1} alignItems="flex-start">
              <TextField
                type="password"
                size="small"
                label={keySet ? "Replace API key" : "API key"}
                placeholder={keySet ? "•••••••• (set)" : "sk-…"}
                value={keyInput}
                onChange={(e): void => setKeyInput(e.target.value)}
                autoComplete="off"
                fullWidth
              />
              <Button
                variant="contained"
                onClick={saveKey}
                disabled={keyInput.trim().length === 0}
                sx={{ flexShrink: 0, mt: 0.25 }}
              >
                Save
              </Button>
            </Stack>

            <TextField
              select
              size="small"
              label="Model"
              value={model}
              onChange={(e): void => setInferenceConfig("deepseek", e.target.value, ds?.params)}
              fullWidth
            >
              {DEEPSEEK_MODELS.map((m) => (
                <MenuItem key={m.id} value={m.id}>
                  {m.label}
                </MenuItem>
              ))}
            </TextField>

            <Stack direction="row" spacing={1} alignItems="center">
              <Button
                variant="outlined"
                size="small"
                onClick={(): void => {
                  setTesting(true);
                  runInferenceProbe("deepseek");
                }}
                disabled={!keySet || testing}
                startIcon={testing ? <CircularProgress size={14} color="inherit" /> : undefined}
              >
                {testing ? "Testing…" : "Test connection"}
              </Button>
              {!testing && probe?.provider === "deepseek" && (
                <Typography variant="caption" color={probe.ok ? "success.main" : "error.main"}>
                  {probe.ok
                    ? `✓ responded (cache hit ${probe.cacheHit} / miss ${probe.cacheMiss})`
                    : `✗ ${probe.error ?? "failed"}`}
                </Typography>
              )}
            </Stack>
          </Stack>
        </Box>

        {/* Skills — provider-agnostic capability units run at turn-end. Each is
            expandable to show the exact prompt + how the output is extracted, so
            the judgment logic is inspectable (not a black box). */}
        <Box>
          <Typography variant="overline" color="text.secondary">
            Skills
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
            Run after each turn to classify what the agent did. The inference provider above powers them.
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
    </Sheet>
  );
}
