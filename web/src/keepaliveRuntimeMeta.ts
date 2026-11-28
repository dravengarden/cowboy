import type { DeepseekCacheProtectionStatus } from "./ComposerSheet";

export interface KeepaliveRuntimeMeta {
  intervalSummary: string;
  scheduleSummary: string | null;
  nextRefreshIn: string | null;
  attemptsLabel: string | null;
  capRemaining: string | null;
  maxAttempts: number;
  maxAgeLabel: string;
}

export function formatShortIdleMinutes(minutes: number): string {
  if (minutes >= 60 && minutes % 60 === 0) return `${minutes / 60}h`;
  return `${minutes}m`;
}

export function formatShortRemainingMs(remainingMs: number): string {
  if (remainingMs <= 0) return "0m";
  const mins = Math.ceil(remainingMs / 60_000);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  const rem = mins % 60;
  return rem === 0 ? `${hours}h` : `${hours}h ${rem}m`;
}

function formatDurationMs(ms: number | undefined): string {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms <= 0) return "";
  const totalMinutes = Math.max(1, Math.round(ms / 60_000));
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours === 0) return `${minutes}m`;
  return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
}

/** Derive human-readable DeepSeek gateway keepalive cadence, next-refresh countdown,
 *  attempt budget (`attempts / max`), and dynamic cap remaining time. */
export function formatKeepaliveRuntimeMeta(
  status: DeepseekCacheProtectionStatus | null | undefined,
  nowMs: number = Date.now(),
  baseIntervalFallback = "5h30m",
): KeepaliveRuntimeMeta | null {
  if (!status) return null;
  const baseLabel = formatDurationMs(status.base_interval_ms) || baseIntervalFallback;
  const scheduledLabel = formatDurationMs(status.scheduled_interval_ms);
  const adaptiveLabel = formatDurationMs(status.adaptive_interval_ms);
  const intervalSummary =
    status.state === "protected"
      ? `Base ${baseLabel} · interval ${scheduledLabel || adaptiveLabel || "learning"}`
      : `Base ${baseLabel} · adaptive target ${adaptiveLabel || "learning"}`;

  const maxAttempts =
    typeof status.maximum_attempts === "number" &&
    Number.isFinite(status.maximum_attempts) &&
    status.maximum_attempts > 0
      ? Math.floor(status.maximum_attempts)
      : 6;
  const maxAgeLabel = formatDurationMs(status.maximum_age_ms) || "26h";

  if (status.state !== "protected") {
    return {
      intervalSummary,
      scheduleSummary: null,
      nextRefreshIn: null,
      attemptsLabel: null,
      capRemaining: null,
      maxAttempts,
      maxAgeLabel,
    };
  }

  const nextRefreshIn =
    typeof status.next_attempt_at_ms === "number" && Number.isFinite(status.next_attempt_at_ms)
      ? status.next_attempt_at_ms > nowMs
        ? formatShortRemainingMs(status.next_attempt_at_ms - nowMs)
        : "due now"
      : null;
  const attemptsLabel =
    typeof status.attempts === "number" && Number.isFinite(status.attempts)
      ? `${status.attempts}/${maxAttempts} refreshes`
      : null;
  const capRemaining =
    typeof status.expires_at_ms === "number" && Number.isFinite(status.expires_at_ms)
      ? status.expires_at_ms > nowMs
        ? `cap ${formatShortRemainingMs(status.expires_at_ms - nowMs)} left`
        : "cap reached"
      : null;

  const parts = [
    nextRefreshIn
      ? nextRefreshIn === "due now"
        ? "Next refresh due"
        : `Next refresh in ${nextRefreshIn}`
      : null,
    attemptsLabel,
    capRemaining,
  ].filter((part): part is string => part !== null);

  return {
    intervalSummary,
    scheduleSummary: parts.length > 0 ? parts.join(" · ") : null,
    nextRefreshIn,
    attemptsLabel,
    capRemaining,
    maxAttempts,
    maxAgeLabel,
  };
}

/** Compact suffix for the mobile ComposerSheet cache-protection line. */
export function keepaliveScheduleSuffix(
  status: DeepseekCacheProtectionStatus | null | undefined,
  baseIntervalFallback = "5h30m",
): string {
  const meta = formatKeepaliveRuntimeMeta(status, Date.now(), baseIntervalFallback);
  return meta?.scheduleSummary ? ` · ${meta.scheduleSummary}` : "";
}
