import {
  providerUsageErrorMessage,
  type ProviderUsage,
  shortResetTime,
  topBarUsageLimits,
} from "./usageLimits";
import { usageWidgetForAccount } from "./usageWidget";

/** Session-sheet queue and page-view row. These stay off the first
 *  screen; the collapsed label must still name the live state. */
export function workspaceOptionsSummary(input: {
  queuePaused: boolean;
  pageView: boolean;
}): string {
  return [
    input.queuePaused ? "Queue paused" : "Queue running",
    input.pageView ? "Page view" : "Conversation",
  ].join(" · ");
}

export function sessionProviderNeedsAttention(input: {
  catalogReady: boolean;
  required: boolean;
  ready: boolean;
}): boolean {
  if (!input.catalogReady) return true;
  return input.required && !input.ready;
}

export function sessionProviderFacts(input: {
  vendor: string;
  version: string;
  accountLabel?: string;
}): readonly { label: string; value: string; mono?: boolean }[] {
  const rows: { label: string; value: string; mono?: boolean }[] = [
    { label: "Vendor", value: input.vendor },
    { label: "Version", value: input.version, mono: true },
  ];
  if (input.accountLabel) {
    rows.push({ label: "Account", value: input.accountLabel });
  }
  return rows;
}

export function sessionProviderManageLabel(
  presentation: "account" | "api_key" | "none",
): string {
  return presentation === "api_key" ? "Manage API key" : "Manage account";
}

export function sessionProviderSummary(input: {
  displayName: string;
  required: boolean;
  ready: boolean;
  accountLabel?: string;
  presentation: "account" | "api_key" | "none";
}): string {
  if (!input.required) return `${input.displayName} · no sign-in`;
  if (!input.ready) {
    return input.presentation === "api_key"
      ? `${input.displayName} · API key missing`
      : `${input.displayName} · signed out`;
  }
  const state = input.presentation === "api_key"
    ? "API key configured"
    : "signed in";
  return input.accountLabel
    ? `${input.displayName} · ${state} · ${input.accountLabel}`
    : `${input.displayName} · ${state}`;
}

export function sessionProviderShowsUsage(input: {
  ready: boolean;
  accountUsageProvider?: string;
}): boolean {
  return input.ready && Boolean(input.accountUsageProvider);
}

export type SessionProviderUsageRow = {
  label: string;
  value: string;
  detail?: string;
};

function compactCny(value: number): string {
  return `¥${value < 0.01 ? value.toFixed(3) : value.toFixed(2)}`;
}

/** Compact account windows for the session Provider facts list.
 *  DeepSeek exposes balance rather than a remaining-percent window. */
export function sessionProviderUsageRows(
  usage: ProviderUsage | undefined,
): SessionProviderUsageRow[] {
  if (!usage) return [];
  const widget = usageWidgetForAccount(usage);
  if (widget?.kind === "deepseek") {
    return [
      { label: "Balance", value: compactCny(widget.balanceCny) },
      { label: "24h spend", value: compactCny(widget.spend24hCny) },
    ];
  }
  return topBarUsageLimits(usage).map((limit) => ({
    label: limit.label,
    value: `${String(limit.remaining)}% remaining`,
    ...(limit.resetsAt === undefined
      ? {}
      : { detail: `Resets ${shortResetTime(limit.resetsAt)}` }),
  }));
}

export function sessionProviderUsageEmptyMessage(
  usage: ProviderUsage | undefined,
): string {
  return providerUsageErrorMessage(
    usage,
    "This account has not exposed usage limits.",
  );
}
