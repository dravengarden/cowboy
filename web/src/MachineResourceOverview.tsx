import { Box, Stack, Typography } from "@mui/material";
import {
  CapacityMeter,
  MetricDistribution,
  MetricFacts,
  type MetricSegment,
} from "./InfoMetricVisuals";
import { formatBytes, formatUptime } from "./machineResources";
import type { MaintenanceMachine } from "./SessionMaintenancePanel";

export function MachineResourceOverview({
  row,
}: {
  row: MaintenanceMachine;
}): React.JSX.Element {
  const { machine } = row;
  const resources = machine.resources;
  const other = Math.max(
    0,
    row.sessions - row.working - row.warm - row.sleeping - row.starting,
  );
  const segments: MetricSegment[] = [
    { label: "working", value: row.working, tone: "info" },
    { label: "warm", value: row.warm, tone: "primary" },
    { label: "sleeping", value: row.sleeping, tone: "neutral" },
    ...(row.starting
      ? [{ label: "starting", value: row.starting, tone: "success" as const }]
      : []),
    ...(other
      ? [{ label: "other", value: other, tone: "neutral" as const }]
      : []),
  ];
  const slots = machine.capacity.max_sessions - machine.active_sessions;
  const tone = machine.connected ? "info" : "neutral";
  return (
    <Stack spacing={1.5}>
      <MetricDistribution
        label={`${row.sessions.toLocaleString()} sessions`}
        segments={segments}
      />
      {row.updating > 0 && (
        <Typography variant="caption" color="info.main">
          {row.updating} Provider update{row.updating === 1 ? "" : "s"} in progress
        </Typography>
      )}
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
          gap: 1.5,
        }}
      >
        {resources && (
          <CapacityMeter
            label="Memory"
            value={
              resources.memory_total_bytes - resources.memory_available_bytes
            }
            total={resources.memory_total_bytes}
            format={formatBytes}
            tone={tone}
            detail={`${formatBytes(resources.memory_available_bytes)} available`}
          />
        )}
        <CapacityMeter
          label="AI workers"
          value={machine.active_sessions}
          total={machine.capacity.max_sessions}
          tone={machine.connected && slots <= 0 ? "warning" : tone}
          detail={
            machine.capacity.draining
              ? "Draining · new starts paused"
              : slots > 0
                ? `${slots.toLocaleString()} slots free`
                : "All slots occupied"
          }
        />
        {resources && (
          <>
            <CapacityMeter
              label="State disk"
              value={
                resources.disk_total_bytes - resources.disk_available_bytes
              }
              total={resources.disk_total_bytes}
              format={formatBytes}
              tone={tone}
              detail={`${formatBytes(resources.disk_available_bytes)} free`}
            />
            <CapacityMeter
              label="Swap"
              value={
                resources.swap_total_bytes > 0
                  ? resources.swap_total_bytes - resources.swap_free_bytes
                  : undefined
              }
              total={resources.swap_total_bytes}
              format={formatBytes}
              tone={tone}
              emptyLabel="Not configured"
            />
          </>
        )}
      </Box>
      {resources && (
        <>
          <MetricFacts
            items={[
              ...(resources.agent_memory_bytes === undefined
                ? []
                : [
                    {
                      label: "Agent memory",
                      value: formatBytes(resources.agent_memory_bytes),
                    },
                  ]),
              ...(resources.memory_headroom_bytes === undefined
                ? []
                : [
                    {
                      label: "Headroom (est.)",
                      value: formatBytes(resources.memory_headroom_bytes),
                    },
                  ]),
              {
                label: "Load · 1 min",
                value: `${(resources.load_1m_milli / 1000).toFixed(2)} · ${resources.cpu_count} CPU${resources.cpu_count === 1 ? "" : "s"}`,
              },
              {
                label: "Uptime",
                value: formatUptime(resources.uptime_seconds),
              },
            ]}
          />
          {resources.memory_headroom_bytes !== undefined && (
            <Typography variant="caption" color="text.secondary">
              Headroom: additional worker memory within runtime limits.
            </Typography>
          )}
        </>
      )}
    </Stack>
  );
}
