import type { ProviderUsage } from "./usageLimits";

/** Service accounts are collected every five minutes; twice that is late. */
export const USAGE_STALE_AFTER_MS = 10 * 60_000;

export type UsageFreshnessTone = "normal" | "stale";

/** How old one account card's numbers are, independent of its siblings. */
export interface UsageFreshness {
  /** Header label beside the refresh button, e.g. `2m ago`. */
  label: string;
  tone: UsageFreshnessTone;
  /** Exact local time of the shown observation, for tooltip and details. */
  exact?: string;
  /** One line under the title when the latest attempt failed but older
   * numbers are still shown, e.g. `Last try failed 3m ago · retrying in 12m`. */
  failure?: string;
}

/** Coarse on purpose: the card clock ticks every 30 s, so no seconds. */
export function freshnessAge(ms: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - ms) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${String(minutes)}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${String(hours)}h ago`;
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function exactUsageTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function hasNumbers(usage: ProviderUsage): boolean {
  return usage.rate_limits !== undefined && usage.rate_limits !== null ||
    usage.account !== undefined && usage.account !== null;
}

/**
 * The time shown is when the displayed numbers were observed, never the last
 * attempt: a failed attempt leaves old numbers, and calling them fresh would
 * mislead. A failure with nothing to show has no age; the card body carries
 * its error instead.
 */
export function usageFreshness(
  usage: ProviderUsage,
  now: number,
  refreshing: boolean,
): UsageFreshness | undefined {
  if (refreshing) return { label: "Refreshing…", tone: "normal" };
  const observed = usage.observed_at_ms;
  if (!Number.isFinite(observed) || observed <= 0) return undefined;
  const failed = typeof usage.error === "string" && usage.error.trim() !== "";
  if (failed && !hasNumbers(usage)) return undefined;
  const sessionOnly = usage.status === "session-only";
  const age = freshnessAge(observed, now);
  const stale = usage.refresh?.stale === true || failed ||
    now - observed > USAGE_STALE_AFTER_MS;
  let failure: string | undefined;
  if (failed && usage.refresh) {
    const parts = [
      `Last try failed ${freshnessAge(usage.refresh.last_attempt_at_ms, now)}`,
    ];
    const retryMinutes = Math.ceil(
      (usage.refresh.next_auto_refresh_at_ms - now) / 60_000,
    );
    if (retryMinutes > 0) parts.push(`retrying in ${String(retryMinutes)}m`);
    failure = parts.join(" · ");
  }
  return {
    label: sessionOnly ? `from session · ${age}` : age,
    // Session-reported numbers age with the session, not the collector.
    tone: stale && !sessionOnly ? "stale" : "normal",
    exact: exactUsageTime(observed),
    ...(failure ? { failure } : {}),
  };
}

/** Minutes until a manual refresh is admitted, or 0 when it may run now. */
export function manualRefreshWaitMinutes(
  usage: ProviderUsage,
  now: number,
): number {
  const after = usage.refresh?.manual_refresh_after_ms ?? 0;
  return after > now ? Math.ceil((after - now) / 60_000) : 0;
}
