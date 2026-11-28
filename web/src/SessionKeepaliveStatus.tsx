import { Chip, Stack, Tooltip, Typography } from "@mui/material";
import { formatCompactTokens } from "./cacheMetricsApi";
import type { DeepseekCacheProtectionStatus } from "./ComposerSheet";
import { CacheProtectionDisableButton } from "./ContextRolloverSettings";
import { formatKeepaliveRuntimeMeta, formatShortIdleMinutes } from "./sessionCacheUi";

/** Desktop-grade DeepSeek gateway keepalive status chip, adaptive cadence,
 *  next-refresh countdown, attempt budget, 26h cap, and revoke button. */
export function BuiltInKeepaliveStatus({
  sessionId,
  status,
  unavailable,
  clockMs,
  rolloverIdleMinutes,
  handoffSupported,
  pendingWork,
  onRevoked,
}: {
  sessionId: string;
  status: DeepseekCacheProtectionStatus | null;
  unavailable: boolean;
  clockMs: number;
  rolloverIdleMinutes: number | null;
  handoffSupported?: boolean | undefined;
  pendingWork?: boolean | undefined;
  onRevoked: () => void;
}): React.JSX.Element | null {
  if (!status && !unavailable) return null;
  const meta = formatKeepaliveRuntimeMeta(status, clockMs);
  const label =
    status?.state === "protected"
      ? status.protected_tokens
        ? `Gateway keepalive · protected ${formatCompactTokens(status.protected_tokens)}`
        : "Gateway keepalive · protected"
      : status?.state === "disabled"
      ? "Gateway keepalive · off"
      : unavailable
      ? "Gateway keepalive · status unavailable"
      : "Gateway keepalive · inactive";
  const color =
    status?.state === "protected"
      ? status.configuredEnabled === false
        ? "warning"
        : "success"
      : unavailable
      ? "warning"
      : "default";
  const tooltip =
    status?.state === "protected"
      ? `${meta?.intervalSummary ?? "Base 5h30m"}. Last verified ${
          status.last_verified_at_ms
            ? new Date(status.last_verified_at_ms).toLocaleString()
            : "recently"
        }. Next attempt ${
          status.next_attempt_at_ms
            ? new Date(status.next_attempt_at_ms).toLocaleString()
            : "scheduled"
        }. Cap expires ${
          status.expires_at_ms
            ? new Date(status.expires_at_ms).toLocaleString()
            : `after ${meta?.maxAgeLabel ?? "26h"}`
        }. Real agent work always preempts it.`
      : status?.state === "disabled"
      ? "Paid DeepSeek gateway keepalive is off for this session (base interval 5h30m)."
      : unavailable
      ? "The owning Machine could not report gateway keepalive status. Agent work is unaffected."
      : "Waiting for a verified ≥90% cache hit over 64K tokens, or previous keepalive snapshot expired.";
  const detailLine =
    status && !unavailable
      ? status.state === "protected"
        ? [meta?.intervalSummary, meta?.scheduleSummary].filter(Boolean).join(" · ")
        : status.configuredEnabled === true && status.state === "inactive"
        ? rolloverIdleMinutes !== null
          ? handoffSupported === false
            ? `${meta?.intervalSummary ?? "Base 5h30m"} · inactive — auto-handoff unavailable on this Machine (hibernate or Clear to renew)`
            : pendingWork === true
            ? `${meta?.intervalSummary ?? "Base 5h30m"} · inactive — hibernation blocked by queued or background work`
            : handoffSupported === true
            ? `${meta?.intervalSummary ?? "Base 5h30m"} · inactive — auto-handoff to ${formatShortIdleMinutes(rolloverIdleMinutes)} idle renewal`
            : `${meta?.intervalSummary ?? "Base 5h30m"} · inactive — ${formatShortIdleMinutes(rolloverIdleMinutes)} idle renewal applies once dormant`
          : `${meta?.intervalSummary ?? "Base 5h30m"} · waiting for ≥64K & ≥90% hit`
        : meta?.intervalSummary ?? null
      : null;
  return (
    <Stack spacing={0.4} alignItems="flex-start">
      <Tooltip title={tooltip}>
        <Chip size="small" variant="outlined" color={color} label={label} />
      </Tooltip>
      {detailLine && (
        <Typography
          variant="caption"
          color="text.secondary"
          data-keepalive-schedule-detail
          sx={{ fontVariantNumeric: "tabular-nums", lineHeight: 1.35 }}
        >
          {detailLine}
        </Typography>
      )}
      <CacheProtectionDisableButton
        sessionId={sessionId}
        status={status}
        onDisabled={onRevoked}
      />
    </Stack>
  );
}
