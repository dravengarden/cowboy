import { LinearProgress, Stack, Typography } from "@mui/material";
import type { ProviderUsageSlotContext } from "./usageLimits";
import { ProviderUsageActivityDetails } from "./ProviderUsageActivityDetails";

/** Host-kit presentation for a `provider.usage` plugin slot. */
export function ProviderUsage({
  context,
}: {
  context: ProviderUsageSlotContext;
}): React.JSX.Element {
  const limits = context.limits ?? [];
  return (
    <Stack
      spacing={1.25}
      data-usage-provider-section={context.provider}
    >
      {context.showTitle !== false && (
        <Typography variant="subtitle2" sx={{ fontWeight: 750 }}>
          {context.title}
        </Typography>
      )}
      {context.refreshLabel && (
        <Typography variant="caption" color="warning.main">
          {context.refreshLabel}
        </Typography>
      )}
      {limits.map((limit) => (
        <Stack key={limit.id} spacing={0.5}>
          <Stack direction="row" justifyContent="space-between">
            <Typography variant="body2">{limit.label}</Typography>
            <Typography variant="body2" fontWeight={750}>
              {`${String(limit.remaining)}% remaining`}
            </Typography>
          </Stack>
          <LinearProgress
            variant="determinate"
            value={limit.remaining}
            sx={{
              height: 6,
              borderRadius: 99,
              "& .MuiLinearProgress-bar": { borderRadius: 99 },
            }}
          />
          {limit.resetsLabel && (
            <Typography variant="caption" color="text.secondary">
              {limit.resetsLabel}
            </Typography>
          )}
        </Stack>
      ))}
      {limits.length === 0 && context.emptyMessage && (
        <Typography variant="body2" color="text.secondary">
          {context.emptyMessage}
        </Typography>
      )}
    </Stack>
  );
}

/** Cowboy-owned detail renderer for usage plugins with activity telemetry. */
export function ProviderUsageActivity({
  context,
}: {
  context: ProviderUsageSlotContext;
}): React.JSX.Element {
  return (
    <Stack spacing={2}>
      <ProviderUsage context={context} />
      {context.showDetails && context.usage && (
        <ProviderUsageActivityDetails usage={context.usage} />
      )}
    </Stack>
  );
}
