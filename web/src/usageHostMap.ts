import { isPluginIdentifier } from "./pluginHost/identity";

export type UsageActivityAgent = {
  id: string;
  label: string;
};

export type UsageCacheProtection = {
  minHitTokens: number;
  minHitLabel: string;
  intervalMs: number;
  intervalLabel: string;
  optionName: string;
  optionDescription: string;
  optionOn: string;
  optionOff: string;
};

export type UsageLimitLabel = {
  id: string;
  label: string;
  windowMinutes?: number;
};

type UsageMaps = {
  plugins: Record<string, string>;
  resets: Record<string, string>;
  products: Record<string, string>;
  parsers: Record<string, string>;
  errors: Record<string, string>;
  orders: Record<string, number>;
  windows: Record<string, number[]>;
  widgets: Record<string, string>;
  empties: Record<string, string>;
  statuses: Record<string, string>;
  omits: Record<string, boolean>;
  shapes: Record<string, string>;
  widgetWindows: Record<string, number>;
  authCopy: Record<string, string>;
  configCopy: Record<string, string>;
  fetchCopy: Record<string, string>;
  prefixes: Record<string, string>;
  labelRows: Record<string, UsageLimitLabel[]>;
  balanceLabels: Record<string, string>;
  spendLabels: Record<string, string>;
  activityAgents: Record<string, UsageActivityAgent[]>;
  activityModels: Record<string, UsageActivityAgent[]>;
  cacheProtection: Record<string, UsageCacheProtection>;
};

function parseCacheProtection(
  value: unknown,
): UsageCacheProtection | undefined {
  if (value === null || typeof value !== "object") return undefined;
  const row = value as {
    min_hit_tokens?: unknown;
    min_hit_label?: unknown;
    interval_ms?: unknown;
    interval_label?: unknown;
    option_name?: unknown;
    option_description?: unknown;
    option_on?: unknown;
    option_off?: unknown;
  };
  if (
    typeof row.min_hit_tokens !== "number" ||
    !Number.isFinite(row.min_hit_tokens) ||
    row.min_hit_tokens <= 0 ||
    typeof row.min_hit_label !== "string" ||
    typeof row.interval_ms !== "number" ||
    !Number.isFinite(row.interval_ms) ||
    row.interval_ms <= 0 ||
    typeof row.interval_label !== "string" ||
    typeof row.option_name !== "string" ||
    typeof row.option_description !== "string" ||
    typeof row.option_on !== "string" ||
    typeof row.option_off !== "string"
  ) {
    return undefined;
  }
  return {
    minHitTokens: row.min_hit_tokens,
    minHitLabel: row.min_hit_label,
    intervalMs: row.interval_ms,
    intervalLabel: row.interval_label,
    optionName: row.option_name,
    optionDescription: row.option_description,
    optionOn: row.option_on,
    optionOff: row.option_off,
  };
}

function readUsage(host: object): {
  account?: string;
  reset?: string;
  product?: string;
  parser?: string;
  error?: string;
  order?: number;
  topBarWindows?: number[];
  widget?: string;
  empty?: string;
  availableStatus?: string;
  omitEmptyLimits?: boolean;
  widgetShape?: string;
  widgetWindow?: number;
  errorAuth?: string;
  errorConfig?: string;
  errorFetch?: string;
  limitIdPrefix?: string;
  limitLabels?: UsageLimitLabel[];
  widgetBalanceLabel?: string;
  widgetSpendLabel?: string;
  activityAgents?: UsageActivityAgent[];
  activityModels?: UsageActivityAgent[];
  cacheProtection?: UsageCacheProtection;
} {
  const record = host as {
    usage?: {
      account?: unknown;
      reset?: unknown;
      product?: unknown;
      parser?: unknown;
      error?: unknown;
      order?: unknown;
      top_bar_windows?: unknown;
      widget?: unknown;
      empty?: unknown;
      available_status?: unknown;
      omit_empty_limits?: unknown;
      widget_shape?: unknown;
      widget_window?: unknown;
      error_auth?: unknown;
      error_config?: unknown;
      error_fetch?: unknown;
      limit_id_prefix?: unknown;
      limit_labels?: unknown;
      widget_balance_label?: unknown;
      widget_spend_label?: unknown;
      activity_agents?: unknown;
      activity_models?: unknown;
      cache_protection?: unknown;
    };
  };
  const usage = record.usage;
  const account = typeof usage?.account === "string"
    ? usage.account
    : undefined;
  const resetValue = usage?.reset;
  const reset = isPluginIdentifier(resetValue) ? resetValue : undefined;
  const product = typeof usage?.product === "string"
    ? usage.product
    : undefined;
  const parserValue = usage?.parser;
  const parser = isPluginIdentifier(parserValue) ? parserValue : undefined;
  const errorValue = usage?.error;
  const error = isPluginIdentifier(errorValue) ? errorValue : undefined;
  const order = typeof usage?.order === "number" &&
      Number.isInteger(usage.order) && usage.order >= 0
    ? usage.order
    : undefined;
  const topBarWindows = Array.isArray(usage?.top_bar_windows)
    ? usage.top_bar_windows.filter((
      minutes,
    ): minutes is number =>
      typeof minutes === "number" && Number.isFinite(minutes) && minutes > 0
    )
    : undefined;
  const widgetValue = usage?.widget;
  const widget = isPluginIdentifier(widgetValue) ? widgetValue : undefined;
  const empty = typeof usage?.empty === "string" ? usage.empty : undefined;
  const availableStatus = typeof usage?.available_status === "string"
    ? usage.available_status
    : undefined;
  const omitEmptyLimits = usage?.omit_empty_limits === true
    ? true
    : usage?.omit_empty_limits === false
    ? false
    : undefined;
  const widgetShape = typeof usage?.widget_shape === "string"
    ? usage.widget_shape
    : undefined;
  const widgetWindow = typeof usage?.widget_window === "number" &&
      Number.isFinite(usage.widget_window) &&
      usage.widget_window > 0
    ? usage.widget_window
    : undefined;
  const errorAuth = typeof usage?.error_auth === "string"
    ? usage.error_auth
    : undefined;
  const errorConfig = typeof usage?.error_config === "string"
    ? usage.error_config
    : undefined;
  const errorFetch = typeof usage?.error_fetch === "string"
    ? usage.error_fetch
    : undefined;
  const limitIdPrefix = typeof usage?.limit_id_prefix === "string"
    ? usage.limit_id_prefix
    : undefined;
  const limitLabels = Array.isArray(usage?.limit_labels)
    ? usage.limit_labels.flatMap((entry): UsageLimitLabel[] => {
      if (entry === null || typeof entry !== "object") return [];
      const label = entry as {
        id?: unknown;
        label?: unknown;
        window_minutes?: unknown;
      };
      if (typeof label.id !== "string" || typeof label.label !== "string") {
        return [];
      }
      const windowMinutes = typeof label.window_minutes === "number" &&
          Number.isFinite(label.window_minutes) && label.window_minutes > 0
        ? label.window_minutes
        : undefined;
      return [{
        id: label.id,
        label: label.label,
        ...(windowMinutes === undefined ? {} : { windowMinutes }),
      }];
    })
    : undefined;
  const widgetBalanceLabel = typeof usage?.widget_balance_label === "string"
    ? usage.widget_balance_label
    : undefined;
  const widgetSpendLabel = typeof usage?.widget_spend_label === "string"
    ? usage.widget_spend_label
    : undefined;
  const activityAgents = Array.isArray(usage?.activity_agents)
    ? usage.activity_agents.flatMap((entry): UsageActivityAgent[] => {
      if (entry === null || typeof entry !== "object") return [];
      const agent = entry as { id?: unknown; label?: unknown };
      if (typeof agent.id !== "string" || typeof agent.label !== "string") {
        return [];
      }
      return [{ id: agent.id, label: agent.label }];
    })
    : undefined;
  const activityModels = Array.isArray(usage?.activity_models)
    ? usage.activity_models.flatMap((entry): UsageActivityAgent[] => {
      if (entry === null || typeof entry !== "object") return [];
      const model = entry as { id?: unknown; label?: unknown };
      if (typeof model.id !== "string" || typeof model.label !== "string") {
        return [];
      }
      return [{ id: model.id, label: model.label }];
    })
    : undefined;
  const cacheProtection = parseCacheProtection(usage?.cache_protection);
  return {
    ...(account ? { account } : {}),
    ...(reset ? { reset } : {}),
    ...(product ? { product } : {}),
    ...(parser ? { parser } : {}),
    ...(error ? { error } : {}),
    ...(order === undefined ? {} : { order }),
    ...(topBarWindows && topBarWindows.length > 0 ? { topBarWindows } : {}),
    ...(widget ? { widget } : {}),
    ...(empty ? { empty } : {}),
    ...(availableStatus ? { availableStatus } : {}),
    ...(omitEmptyLimits === undefined ? {} : { omitEmptyLimits }),
    ...(widgetShape ? { widgetShape } : {}),
    ...(widgetWindow === undefined ? {} : { widgetWindow }),
    ...(errorAuth ? { errorAuth } : {}),
    ...(errorConfig ? { errorConfig } : {}),
    ...(errorFetch ? { errorFetch } : {}),
    ...(limitIdPrefix ? { limitIdPrefix } : {}),
    ...(limitLabels && limitLabels.length > 0 ? { limitLabels } : {}),
    ...(widgetBalanceLabel ? { widgetBalanceLabel } : {}),
    ...(widgetSpendLabel ? { widgetSpendLabel } : {}),
    ...(activityAgents && activityAgents.length > 0 ? { activityAgents } : {}),
    ...(activityModels && activityModels.length > 0 ? { activityModels } : {}),
    ...(cacheProtection ? { cacheProtection } : {}),
  };
}

function emptyUsageMaps(): UsageMaps {
  return {
    plugins: {},
    resets: {},
    products: {},
    parsers: {},
    errors: {},
    orders: {},
    windows: {},
    widgets: {},
    empties: {},
    statuses: {},
    omits: {},
    shapes: {},
    widgetWindows: {},
    authCopy: {},
    configCopy: {},
    fetchCopy: {},
    prefixes: {},
    labelRows: {},
    balanceLabels: {},
    spendLabels: {},
    activityAgents: {},
    activityModels: {},
    cacheProtection: {},
  };
}

function collectUsageMaps(hosts: unknown): UsageMaps {
  const maps = emptyUsageMaps();
  if (!Array.isArray(hosts)) return maps;
  for (const host of hosts) {
    if (host === null || typeof host !== "object") continue;
    const defaultForId = (host as { default_for_id?: unknown }).default_for_id;
    if (defaultForId !== undefined && defaultForId !== true) continue;
    const id = (host as { id?: unknown }).id;
    const usage = readUsage(host);
    const account = usage.account;
    if (!isPluginIdentifier(id) || !isPluginIdentifier(account)) continue;
    maps.plugins[account] = id;
    if (usage.reset) maps.resets[account] = usage.reset;
    if (usage.product) maps.products[account] = usage.product;
    if (usage.parser) maps.parsers[account] = usage.parser;
    if (usage.error) maps.errors[account] = usage.error;
    if (usage.order !== undefined) maps.orders[account] = usage.order;
    if (usage.topBarWindows) maps.windows[account] = usage.topBarWindows;
    if (usage.widget) maps.widgets[account] = usage.widget;
    if (usage.empty) maps.empties[account] = usage.empty;
    if (usage.availableStatus) {
      maps.statuses[account] = usage.availableStatus;
    }
    if (usage.omitEmptyLimits !== undefined) {
      maps.omits[account] = usage.omitEmptyLimits;
    }
    if (usage.widgetShape) maps.shapes[account] = usage.widgetShape;
    if (usage.widgetWindow !== undefined) {
      maps.widgetWindows[account] = usage.widgetWindow;
    }
    if (usage.errorAuth) maps.authCopy[account] = usage.errorAuth;
    if (usage.errorConfig) maps.configCopy[account] = usage.errorConfig;
    if (usage.errorFetch) maps.fetchCopy[account] = usage.errorFetch;
    if (usage.limitIdPrefix) maps.prefixes[account] = usage.limitIdPrefix;
    if (usage.limitLabels) maps.labelRows[account] = usage.limitLabels;
    if (usage.widgetBalanceLabel) {
      maps.balanceLabels[account] = usage.widgetBalanceLabel;
    }
    if (usage.widgetSpendLabel) {
      maps.spendLabels[account] = usage.widgetSpendLabel;
    }
    if (usage.activityAgents) {
      maps.activityAgents[account] = usage.activityAgents;
    }
    if (usage.activityModels) {
      maps.activityModels[account] = usage.activityModels;
    }
    if (usage.cacheProtection) {
      maps.cacheProtection[account] = usage.cacheProtection;
    }
  }
  return maps;
}

let overlayUsage = emptyUsageMaps();

/** Replace account capability data from the activated, validated host inventory. */
export function applyUsageHostPlugins(hosts: unknown): void {
  overlayUsage = collectUsageMaps(hosts);
}

/** Map account-usage provider ids onto Agent Plugin slot ids. */
export function usagePluginId(provider: string): string {
  return overlayUsage.plugins[provider] ?? provider;
}

/** Map account-usage provider ids onto usage-reset API ids. */
export function usageResetId(provider: string): string | undefined {
  return overlayUsage.resets[provider];
}

/** Display name for an account-usage provider id. */
export function usageProductLabel(provider: string): string {
  return overlayUsage.products[provider] ?? provider;
}

/** Opaque usage-limit capability declared by the account's host plugin. */
export function usageLimitParser(provider: string): string {
  return overlayUsage.parsers[provider] ?? "generic-buckets";
}

/** Opaque usage-error capability declared by the account's host plugin. */
export function usageErrorKind(provider: string): string {
  return overlayUsage.errors[provider] ?? "raw";
}

/** First-party card order. Unknown accounts stay insertion-stable. */
export function usageCardOrder(provider: string): number {
  return overlayUsage.orders[provider] ?? Number.MAX_SAFE_INTEGER;
}

/** Optional top-bar window minutes declared by the account's host plugin. */
export function usageTopBarWindowMinutes(
  provider: string,
): number[] | undefined {
  const windows = overlayUsage.windows[provider];
  return windows && windows.length > 0 ? windows : undefined;
}

/** Opaque desktop-widget kind declared by the account's host plugin. */
export function usageWidgetKind(provider: string): string {
  return overlayUsage.widgets[provider] ?? "none";
}

/** Optional empty-state copy declared by the account's host plugin. */
export function usageEmptyMessage(provider: string): string | undefined {
  return overlayUsage.empties[provider];
}

/** Badge shown when the account is available and has no plan name. */
export function usageAvailableStatus(provider: string): string | undefined {
  return overlayUsage.statuses[provider];
}

/** Whether the host should skip the generic empty-limit fallback copy. */
export function usageOmitEmptyLimits(provider: string): boolean {
  return overlayUsage.omits[provider] === true;
}

/** Compact desktop-widget shape declared by the account's host plugin. */
export function usageWidgetShape(
  provider: string,
): "percent" | "balance" | "none" {
  const shape = overlayUsage.shapes[provider];
  return shape === "percent" || shape === "balance" ? shape : "none";
}

/** Optional compact-widget window minutes declared by the account's host plugin. */
export function usageWidgetWindow(provider: string): number | undefined {
  return overlayUsage.widgetWindows[provider];
}

/** Auth-failure copy declared by the account's host plugin. */
export function usageErrorAuth(provider: string): string | undefined {
  return overlayUsage.authCopy[provider];
}

/** Configuration-failure copy declared by the account's host plugin. */
export function usageErrorConfig(provider: string): string | undefined {
  return overlayUsage.configCopy[provider];
}

/** Fetch-failure copy declared by the account's host plugin. */
export function usageErrorFetch(provider: string): string | undefined {
  return overlayUsage.fetchCopy[provider];
}

/** Prefix for parser-owned limit row ids, e.g. claude-five_hour. */
export function usageLimitIdPrefix(provider: string): string {
  return overlayUsage.prefixes[provider] ?? provider;
}

/** Presentation for one parser-owned rate-limit type. */
export function usageLimitLabel(
  provider: string,
  kind: string,
): { label: string; windowMinutes?: number } {
  const labels = overlayUsage.labelRows[provider] ?? [];
  const match = labels.find((entry) => entry.id === kind);
  if (!match) return { label: "Plan usage" };
  return {
    label: match.label,
    ...(match.windowMinutes === undefined
      ? {}
      : { windowMinutes: match.windowMinutes }),
  };
}

/** Stable id for a parser-owned limit row. */
export function usageLimitRowId(provider: string, kind: string): string {
  return `${usageLimitIdPrefix(provider)}-${kind}`;
}

/** Compact-widget balance row label declared by the account's host plugin. */
export function usageWidgetBalanceLabel(provider: string): string {
  return overlayUsage.balanceLabels[provider] ?? "Balance";
}

/** Compact-widget spend row label declared by the account's host plugin. */
export function usageWidgetSpendLabel(provider: string): string {
  return overlayUsage.spendLabels[provider] ?? "24h spend";
}

function uniqueAgents(rows: UsageActivityAgent[][]): UsageActivityAgent[] {
  const seen = new Set<string>();
  const agents: UsageActivityAgent[] = [];
  for (const row of rows) {
    for (const agent of row) {
      if (seen.has(agent.id)) continue;
      seen.add(agent.id);
      agents.push(agent);
    }
  }
  return agents;
}

/** Telemetry agent lanes declared by host plugins. Omit provider to merge all. */
export function usageActivityAgents(
  provider?: string,
): UsageActivityAgent[] {
  if (provider) {
    return overlayUsage.activityAgents[provider] ?? [];
  }
  return uniqueAgents(Object.values(overlayUsage.activityAgents));
}

/** Telemetry agent ids declared by host plugins. */
export function usageActivityAgentIds(provider?: string): string[] {
  return usageActivityAgents(provider).map((agent) => agent.id);
}

/** Display name for a telemetry agent lane. */
export function usageActivityAgentLabel(
  agent: string,
  provider?: string,
): string {
  return usageActivityAgents(provider).find((entry) => entry.id === agent)
    ?.label ?? agent;
}

/** Telemetry model families declared by host plugins. Omit provider to merge all. */
export function usageActivityModels(
  provider?: string,
): UsageActivityAgent[] {
  if (provider) {
    return overlayUsage.activityModels[provider] ?? [];
  }
  return uniqueAgents(Object.values(overlayUsage.activityModels));
}

/** Telemetry model-family ids declared by host plugins. */
export function usageActivityModelIds(provider?: string): string[] {
  return usageActivityModels(provider).map((model) => model.id);
}

/** Display name for a telemetry model family. */
export function usageActivityModelLabel(
  model: string,
  provider?: string,
): string {
  return usageActivityModels(provider).find((entry) => entry.id === model)
    ?.label ?? model;
}

function firstCacheProtection(
  provider?: string,
): UsageCacheProtection | undefined {
  if (provider) {
    return overlayUsage.cacheProtection[provider];
  }
  return Object.values(overlayUsage.cacheProtection)[0];
}

/** Prompt-cache protection thresholds declared by host plugins. */
export function usageCacheProtection(
  provider?: string,
): UsageCacheProtection | undefined {
  return firstCacheProtection(provider);
}

/** Minimum verified hit tokens before cache protection is offered. */
export function usageCacheMinHitTokens(provider?: string): number {
  return usageCacheProtection(provider)?.minHitTokens ?? 0;
}

/** Display label for the cache-protection hit threshold. */
export function usageCacheMinHitLabel(provider?: string): string {
  return usageCacheProtection(provider)?.minHitLabel ?? "";
}

/** Base keepalive interval in milliseconds. */
export function usageCacheIntervalMs(provider?: string): number {
  return usageCacheProtection(provider)?.intervalMs ?? 0;
}

/** Display label for the cache-protection base interval. */
export function usageCacheIntervalLabel(provider?: string): string {
  return usageCacheProtection(provider)?.intervalLabel ?? "";
}

/** Session-config option name declared by the account's host plugin. */
export function usageCacheOptionName(provider?: string): string {
  return usageCacheProtection(provider)?.optionName ?? "";
}
