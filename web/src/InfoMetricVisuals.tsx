import type { ReactNode } from "react";
import { Box, darken, LinearProgress, Stack, Typography } from "@mui/material";
import { desktopSize } from "./surface/desktopSize";

export type MetricTone = "primary" | "info" | "success" | "warning" | "neutral";
const toneColor = (tone: MetricTone): string =>
  tone === "neutral" ? "text.disabled" : `${tone}.main`;

export function capacityPercent(
  value: number | undefined,
  total: number | undefined,
): number | undefined {
  if (
    value === undefined ||
    total === undefined ||
    !Number.isFinite(value) ||
    !Number.isFinite(total) ||
    value < 0 ||
    total <= 0
  )
    return undefined;
  const percent = (value / total) * 100;
  return Number.isFinite(percent) ? percent : undefined;
}

export function metricPercentLabel(percent: number): string {
  if (percent > 0 && percent < 1) return "<1%";
  if (percent > 99 && percent < 100) return ">99%";
  return `${Math.round(percent).toLocaleString()}%`;
}

/** A measurement, not a task in progress. Unknown capacity never looks empty. */
export function CapacityMeter({
  label,
  value,
  total,
  format = (n) => n.toLocaleString(),
  detail,
  tone = "info",
  totalLabel,
  emptyLabel = "Not reported",
}: {
  label: string;
  value: number | undefined;
  total?: number | undefined;
  format?: (n: number) => string;
  detail?: ReactNode;
  tone?: MetricTone;
  totalLabel?: string | undefined;
  emptyLabel?: string;
}): React.JSX.Element {
  const known = value !== undefined && Number.isFinite(value) && value >= 0;
  const percent = capacityPercent(value, total);
  const valueLabel = known ? format(value) : emptyLabel;
  return (
    <Stack spacing={0.5} sx={{ minWidth: 0 }} data-capacity-metric={label}>
      <Stack
        direction="row"
        justifyContent="space-between"
        alignItems="baseline"
        gap={1}
      >
        <Typography variant="caption" color="text.secondary">
          {label}
        </Typography>
        {percent !== undefined && (
          <Typography
            variant="caption"
            sx={{ fontVariantNumeric: "tabular-nums", fontWeight: 650 }}
          >
            {metricPercentLabel(percent)}
          </Typography>
        )}
      </Stack>
      <Box
        sx={{
          display: "flex",
          alignItems: "baseline",
          flexWrap: "wrap",
          columnGap: 0.5,
          fontVariantNumeric: "tabular-nums",
        }}
      >
        <Typography
          component="span"
          sx={{ fontSize: desktopSize(17), fontWeight: 650, lineHeight: 1.35 }}
        >
          {valueLabel}
        </Typography>
        {total !== undefined && Number.isFinite(total) && total > 0 && (
          <Typography variant="caption" color="text.secondary" component="span">
            / {totalLabel ?? format(total)}
          </Typography>
        )}
      </Box>
      {percent !== undefined && (
        <LinearProgress
          variant="determinate"
          value={Math.min(percent, 100)}
          role="meter"
          aria-label={label}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.min(percent, 100)}
          aria-valuetext={`${valueLabel} of ${totalLabel ?? format(total!)} · ${metricPercentLabel(percent)}`}
          sx={{
            height: 7,
            borderRadius: 99,
            bgcolor: "action.selected",
            "& .MuiLinearProgress-bar": {
              bgcolor: toneColor(tone),
              borderRadius: 99,
              transition: "none",
            },
          }}
        />
      )}
      {detail && (
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ overflowWrap: "anywhere" }}
        >
          {detail}
        </Typography>
      )}
    </Stack>
  );
}

export function QuotaMeter({
  label,
  remaining,
  resetsLabel,
}: {
  label: string;
  remaining: number;
  resetsLabel?: string | undefined;
}): React.JSX.Element {
  const percent = capacityPercent(remaining, 100);
  return (
    <Stack spacing={0.5}>
      <Stack direction="row" justifyContent="space-between" gap={1}>
        <Typography variant="body2">{label}</Typography>
        <Typography
          variant="body2"
          fontWeight={650}
          sx={{ fontVariantNumeric: "tabular-nums", textAlign: "right" }}
        >
          {percent === undefined
            ? "Not reported"
            : `${metricPercentLabel(percent)} remaining`}
        </Typography>
      </Stack>
      {percent !== undefined && (
        <LinearProgress
          variant="determinate"
          value={Math.min(percent, 100)}
          role="meter"
          aria-label={label}
          aria-valuetext={`${metricPercentLabel(percent)} remaining`}
          sx={{
            height: 7,
            borderRadius: 99,
            bgcolor: "action.selected",
            "& .MuiLinearProgress-bar": {
              borderRadius: 99,
              transition: "none",
            },
          }}
        />
      )}
      {resetsLabel && (
        <Typography variant="caption" color="text.secondary">
          {resetsLabel}
        </Typography>
      )}
    </Stack>
  );
}

export function MetricFacts({
  items,
  columns = 2,
}: {
  columns?: 2 | 3;
  items: readonly { label: string; value: ReactNode; tone?: MetricTone }[];
}): React.JSX.Element {
  return (
    <Box
      component="dl"
      sx={{
        m: 0,
        display: "grid",
        gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
        gap: 1,
      }}
    >
      {items.map(({ label, value, tone }) => (
        <Box key={label} sx={{ minWidth: 0 }}>
          <Typography component="dt" variant="caption" color="text.secondary">
            {label}
          </Typography>
          <Typography
            component="dd"
            variant="body2"
            sx={(theme) => ({
              m: 0,
              fontWeight: 600,
              fontVariantNumeric: "tabular-nums",
              overflowWrap: "anywhere",
              ...(tone ? { color: tone === "warning" ? theme.palette.mode === "dark" ? theme.palette.warning.light : darken(theme.palette.warning.main, 0.3) : toneColor(tone) } : {}),
            })}
          >
            {value}
          </Typography>
        </Box>
      ))}
    </Box>
  );
}

export interface MetricSegment {
  label: string;
  value: number;
  tone: MetricTone;
}

/** Only mutually exclusive populations belong in this chart; overlays stay outside. */
export function MetricDistribution({
  label,
  segments,
}: {
  label: string;
  segments: readonly MetricSegment[];
}): React.JSX.Element {
  const valid = segments.filter(
    (segment) => Number.isFinite(segment.value) && segment.value >= 0,
  );
  const total = valid.reduce((sum, segment) => sum + segment.value, 0);
  return (
    <Stack spacing={0.75} data-metric-distribution={label}>
      <Typography variant="body2" fontWeight={600}>
        {label}
      </Typography>
      {total > 0 && (
        <Box
          role="img"
          aria-label={valid
            .map(
              (segment) => `${segment.value.toLocaleString()} ${segment.label}`,
            )
            .join(", ")}
          sx={{
            display: "flex",
            height: 8,
            overflow: "hidden",
            borderRadius: 99,
            bgcolor: "action.selected",
          }}
        >
          {valid
            .filter((segment) => segment.value > 0)
            .map((segment) => (
              <Box
                key={segment.label}
                sx={{
                  width: `${(segment.value / total) * 100}%`,
                  bgcolor: toneColor(segment.tone),
                }}
              />
            ))}
        </Box>
      )}
      <Box sx={{ display: "flex", flexWrap: "wrap", gap: "4px 14px" }}>
        {valid.filter((segment) => total === 0 || segment.value > 0).map((segment) => (
          <Stack
            key={segment.label}
            direction="row"
            alignItems="center"
            spacing={0.5}
          >
            <Box
              aria-hidden
              sx={{
                width: 6,
                height: 6,
                borderRadius: "50%",
                bgcolor: toneColor(segment.tone),
              }}
            />
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ fontVariantNumeric: "tabular-nums" }}
            >
              {segment.value.toLocaleString()} {segment.label}
            </Typography>
          </Stack>
        ))}
      </Box>
    </Stack>
  );
}
