import { useEffect, useState } from "react";
import { Alert, Box, Button, Divider, Stack, Typography } from "@mui/material";
import {
  CapacityMeter,
  MetricDistribution,
  MetricFacts,
} from "./InfoMetricVisuals";
import {
  type ClientRuntimeMetrics,
  readClientRuntimeMetrics,
} from "./clientRuntimeMetrics";
import { expectHttpOk } from "./httpResponse";
import { formatBytes } from "./machineResources";
import { desktopSize } from "./surface/desktopSize";

export interface ServiceMetrics {
  db_bytes: number;
  events_rows: number;
  sessions_live: number;
  sessions_deleted: number;
  daemon_rss_bytes: number;
  observability_pending: number;
  observability_accepted_batches: number;
  observability_dropped_batches: number;
  observability_failed_log_batches: number;
  observability_failed_metric_batches: number;
  observability_failed_trace_batches?: number;
}

const summarySx = {
  border: 1,
  borderColor: "divider",
  borderRadius: 1.5,
  p: 1.5,
  minWidth: 0,
};

export function ServiceMetricsSummary({
  metrics: m,
}: {
  metrics: ServiceMetrics;
}): React.JSX.Element {
  const exportFailures =
    m.observability_failed_log_batches +
    m.observability_failed_metric_batches +
    (m.observability_failed_trace_batches ?? 0);
  return (
    <Stack spacing={1.5} sx={summarySx}>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
          gap: 1.5,
        }}
      >
        {[
          [
            "Database",
            formatBytes(m.db_bytes),
            `${m.events_rows.toLocaleString()} event rows`,
          ],
          ["Daemon memory", formatBytes(m.daemon_rss_bytes), "Current process"],
        ].map(([label, value, detail]) => (
          <Box key={label} sx={{ minWidth: 0 }}>
            <Typography
              variant="caption"
              color="text.secondary"
              display="block"
            >
              {label}
            </Typography>
            <Typography
              sx={{
                fontSize: desktopSize(20),
                fontWeight: 650,
                fontVariantNumeric: "tabular-nums",
              }}
            >
              {value}
            </Typography>
            <Typography variant="caption" color="text.secondary">
              {detail}
            </Typography>
          </Box>
        ))}
      </Box>
      <Divider />
      <MetricDistribution
        label="Session records"
        segments={[
          { label: "live", value: m.sessions_live, tone: "info" },
          { label: "deleted", value: m.sessions_deleted, tone: "neutral" },
        ]}
      />
      <Typography variant="caption" color="text.secondary">
        Live includes sleeping · Deleted purge: ≤3d
      </Typography>
      <Divider />
      <Stack
        direction="row"
        alignItems="baseline"
        justifyContent="space-between"
        gap={1}
      >
        <Typography variant="body2" fontWeight={600}>
          Telemetry batches
        </Typography>
        <Typography variant="caption" color="text.secondary">
          {m.observability_pending.toLocaleString()} pending now
        </Typography>
      </Stack>
      <MetricFacts
        columns={3}
        items={[
          {
            label: "Accepted",
            value: m.observability_accepted_batches.toLocaleString(),
          },
          {
            label: "Dropped",
            value: m.observability_dropped_batches.toLocaleString(),
            ...(m.observability_dropped_batches > 0
              ? { tone: "warning" as const }
              : {}),
          },
          {
            label: "Export failures",
            value: exportFailures.toLocaleString(),
            ...(exportFailures > 0 ? { tone: "warning" as const } : {}),
          },
        ]}
      />
      <Typography variant="caption" color="text.secondary">
        Cumulative since Service start, not current delivery status.
      </Typography>
    </Stack>
  );
}

export function ServiceStorageInfoSection(): React.JSX.Element {
  const [metrics, setMetrics] = useState<ServiceMetrics | null>(null);
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const ctrl = new AbortController();
    setError(undefined);
    void (async () => {
      try {
        const response = await fetch("/api/metrics", { signal: ctrl.signal });
        await expectHttpOk(response, "Could not load Service metrics");
        const next = (await response.json()) as ServiceMetrics;
        const required = [
          "db_bytes",
          "events_rows",
          "sessions_live",
          "sessions_deleted",
          "daemon_rss_bytes",
          "observability_pending",
          "observability_accepted_batches",
          "observability_dropped_batches",
          "observability_failed_log_batches",
          "observability_failed_metric_batches",
        ] as const;
        if (
          !next ||
          required.some((key) => !Number.isFinite(next[key]) || next[key] < 0)
        )
          throw new Error("Service metrics are unavailable in this response.");
        if (!ctrl.signal.aborted) setMetrics(next);
      } catch (cause) {
        if (!ctrl.signal.aborted)
          setError(cause instanceof Error ? cause.message : String(cause));
      }
    })();
    return () => ctrl.abort();
  }, [attempt]);
  if (error)
    return (
      <Alert
        severity="warning"
        action={
          <Button
            color="inherit"
            onClick={() => setAttempt((value) => value + 1)}
          >
            Retry
          </Button>
        }
      >
        {error}
      </Alert>
    );
  if (!metrics)
    return (
      <Typography role="status" variant="body2" color="text.secondary">
        Loading Service metrics…
      </Typography>
    );
  return <ServiceMetricsSummary metrics={metrics} />;
}

export function ClientMetricsSummary({
  metrics,
}: {
  metrics: ClientRuntimeMetrics;
}): React.JSX.Element {
  return (
    <Stack spacing={1.5} sx={summarySx}>
      <CapacityMeter
        label="App storage used"
        value={metrics.storageUsageBytes}
        total={metrics.storageQuotaBytes}
        format={formatBytes}
        totalLabel={
          metrics.storageQuotaBytes === undefined
            ? undefined
            : `up to ${formatBytes(metrics.storageQuotaBytes)}`
        }
        detail="Browser allowance, not total device disk space."
      />
      {metrics.jsHeapUsedBytes !== undefined && (
        <CapacityMeter
          label="JS heap"
          value={metrics.jsHeapUsedBytes}
          total={metrics.jsHeapLimitBytes}
          format={formatBytes}
        />
      )}
      <Divider />
      <MetricFacts
        items={[
          { label: "Local state", value: formatBytes(metrics.bytes) },
          { label: "Local entries", value: metrics.entries.toLocaleString() },
          { label: "Composer drafts", value: metrics.drafts.toLocaleString() },
          {
            label: "Cache buckets",
            value: metrics.cacheBuckets?.toLocaleString() ?? "Not reported",
          },
        ]}
      />
      <Divider />
      <MetricFacts
        items={[
          { label: "Surface", value: metrics.surface },
          {
            label: "Service worker",
            value:
              metrics.serviceWorker === "Unsupported"
                ? "Not available"
                : metrics.serviceWorker,
          },
        ]}
      />
    </Stack>
  );
}

export function ClientStorageInfoSection(): React.JSX.Element {
  const [metrics, setMetrics] = useState<ClientRuntimeMetrics | null>(null);
  const [error, setError] = useState(false);
  useEffect(() => {
    let active = true;
    void readClientRuntimeMetrics()
      .then((value) => {
        if (active) setMetrics(value);
      })
      .catch(() => {
        if (active) setError(true);
      });
    return () => {
      active = false;
    };
  }, []);
  if (error)
    return (
      <Typography variant="body2" color="text.secondary">
        This device did not expose storage metrics.
      </Typography>
    );
  if (!metrics)
    return (
      <Typography role="status" variant="body2" color="text.secondary">
        Reading device storage…
      </Typography>
    );
  return <ClientMetricsSummary metrics={metrics} />;
}
