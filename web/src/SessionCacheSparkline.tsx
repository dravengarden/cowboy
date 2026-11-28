import { useMemo, useState } from "react";
import { alpha, Box, ButtonBase, Stack, Typography } from "@mui/material";
import type { TurnCacheFact } from "./cacheMetricsApi";
import type { MetricTone } from "./InfoMetricVisuals";
import { buildTurnSparklineBars } from "./sessionCacheUi";

function barToneColor(tone: MetricTone): string {
  return tone === "neutral" ? "text.disabled" : `${tone}.main`;
}

/** Chronological (oldest → newest) turn-by-turn cache hit-rate sparkline with
 *  renewed/compacted/idle-gap markers and tap/hover detail readout. */
export function SessionCacheSparkline({
  turns,
  compact = false,
}: {
  turns: readonly TurnCacheFact[];
  compact?: boolean;
}): React.JSX.Element | null {
  const bars = useMemo(() => buildTurnSparklineBars(turns), [turns]);
  const [pinnedSeq, setPinnedSeq] = useState<number | null>(null);
  const [hoveredSeq, setHoveredSeq] = useState<number | null>(null);

  if (bars.length === 0) return null;

  const activeSeq = hoveredSeq ?? pinnedSeq ?? bars[bars.length - 1]!.seq;
  const activeBar = bars.find((bar) => bar.seq === activeSeq) ?? bars[bars.length - 1]!;
  const chartHeight = compact ? 32 : 40;

  return (
    <Stack spacing={0.5} data-cache-sparkline>
      <Stack direction="row" justifyContent="space-between" alignItems="baseline" spacing={1}>
        <Typography variant="caption" color="text.secondary" fontWeight={600}>
          Hit rate by turn ({bars.length})
        </Typography>
        <Typography variant="caption" color="text.secondary" sx={{ fontSize: "0.68rem" }}>
          oldest → latest
        </Typography>
      </Stack>

      <Box
        role="group"
        aria-label="Prompt cache hit rate across recent turns"
        onMouseLeave={(): void => setHoveredSeq(null)}
        sx={{
          display: "flex",
          alignItems: "flex-end",
          gap: "3px",
          height: chartHeight + 12,
          px: 0.75,
          pt: 1.25,
          pb: 0.5,
          borderRadius: 1.25,
          bgcolor: (theme) => alpha(theme.palette.text.primary, 0.03),
          border: 1,
          borderColor: "divider",
        }}
      >
        {bars.map((bar) => {
          const selected = bar.seq === activeBar.seq;
          const marker = bar.renewed ? "↻" : bar.compacted ? "⊟" : null;
          return (
            <Box
              key={bar.seq}
              sx={{
                flex: "1 1 0",
                minWidth: 6,
                maxWidth: 18,
                height: "100%",
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "flex-end",
                position: "relative",
                ...(bar.longIdleGap
                  ? {
                      borderLeft: "1px dashed",
                      borderColor: (theme) => alpha(theme.palette.warning.main, 0.55),
                      pl: "2px",
                    }
                  : {}),
              }}
            >
              {marker && (
                <Typography
                  component="span"
                  aria-hidden="true"
                  sx={{
                    position: "absolute",
                    top: -11,
                    fontSize: "0.62rem",
                    lineHeight: 1,
                    fontWeight: 700,
                    color: bar.renewed ? "info.main" : "text.secondary",
                    pointerEvents: "none",
                  }}
                >
                  {marker}
                </Typography>
              )}
              <ButtonBase
                aria-label={bar.ariaLabel}
                aria-pressed={selected}
                data-sparkline-turn={bar.seq}
                onMouseEnter={(): void => setHoveredSeq(bar.seq)}
                onFocus={(): void => setHoveredSeq(bar.seq)}
                onBlur={(): void => setHoveredSeq(null)}
                onClick={(): void =>
                  setPinnedSeq((cur) => (cur === bar.seq ? null : bar.seq))
                }
                sx={{
                  width: "100%",
                  height: "100%",
                  display: "flex",
                  alignItems: "flex-end",
                  borderRadius: 0.5,
                  outlineOffset: 1,
                }}
              >
                <Box
                  sx={(theme) => ({
                    width: "100%",
                    height: `${bar.heightPercent}%`,
                    minHeight: 3,
                    borderRadius: "2px 2px 1px 1px",
                    bgcolor:
                      bar.share === null
                        ? alpha(theme.palette.text.disabled, 0.35)
                        : barToneColor(bar.tone),
                    opacity: selected ? 1 : 0.72,
                    boxShadow: selected
                      ? `0 0 0 1.5px ${theme.palette.background.paper}, 0 0 0 2.5px ${theme.palette.primary.main}`
                      : "none",
                    transition: "opacity 120ms ease",
                  })}
                />
              </ButtonBase>
            </Box>
          );
        })}
      </Box>

      <Typography
        variant="caption"
        color="text.secondary"
        data-sparkline-readout
        sx={{
          fontVariantNumeric: "tabular-nums",
          overflowWrap: "anywhere",
          lineHeight: 1.35,
        }}
      >
        {activeBar.summary}
      </Typography>
    </Stack>
  );
}
