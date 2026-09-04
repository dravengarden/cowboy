import { bundledHostPlugins } from "./bundledHostPlugins.ts";

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

function parseCacheProtection(value: unknown): UsageCacheProtection | undefined {
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
    usage_account?: unknown;
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
  const account = typeof record.usage?.account === "string"
    ? record.usage.account
    : typeof record.usage_account === "string"
    ? record.usage_account
    : undefined;
  const reset = typeof record.usage?.reset === "string"
    ? record.usage.reset
    : undefined;
  const product = typeof record.usage?.product === "string"
    ? record.usage.product
    : undefined;
  const parser = typeof record.usage?.parser === "string"
    ? record.usage.parser
    : undefined;
  const error = typeof record.usage?.error === "string"
    ? record.usage.error
    : undefined;
  const order = typeof record.usage?.order === "number" &&
      Number.isInteger(record.usage.order) && record.usage.order >= 0
    ? record.usage.order
    : undefined;
  const topBarWindows = Array.isArray(record.usage?.top_bar_windows)
    ? record.usage.top_bar_windows.filter((
      minutes,
    ): minutes is number =>
      typeof minutes === "number" && Number.isFinite(minutes) && minutes > 0
    )
    : undefined;
  const widget = typeof record.usage?.widget === "string"
    ? record.usage.widget
    : undefined;
  const empty = typeof record.usage?.empty === "string"
    ? record.usage.empty
    : undefined;
  const availableStatus = typeof record.usage?.available_status === "string"
    ? record.usage.available_status
    : undefined;
  const omitEmptyLimits = record.usage?.omit_empty_limits === true
    ? true
    : record.usage?.omit_empty_limits === false
    ? false
    : undefined;
  const widgetShape = typeof record.usage?.widget_shape === "string"
    ? record.usage.widget_shape
    : undefined;
  const widgetWindow = typeof record.usage?.widget_window === "number" &&
      Number.isFinite(record.usage.widget_window) &&
      record.usage.widget_window > 0
    ? record.usage.widget_window
    : undefined;
  const errorAuth = typeof record.usage?.error_auth === "string"
    ? record.usage.error_auth
    : undefined;
  const errorConfig = typeof record.usage?.error_config === "string"
    ? record.usage.error_config
    : undefined;
  const errorFetch = typeof record.usage?.error_fetch === "string"
    ? record.usage.error_fetch
    : undefined;
  const limitIdPrefix = typeof record.usage?.limit_id_prefix === "string"
    ? record.usage.limit_id_prefix
    : undefined;
  const limitLabels = Array.isArray(record.usage?.limit_labels)
    ? record.usage.limit_labels.flatMap((entry): UsageLimitLabel[] => {
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
  const widgetBalanceLabel = typeof record.usage?.widget_balance_label === "string"
    ? record.usage.widget_balance_label
    : undefined;
  const widgetSpendLabel = typeof record.usage?.widget_spend_label === "string"
    ? record.usage.widget_spend_label
    : undefined;
  const activityAgents = Array.isArray(record.usage?.activity_agents)
    ? record.usage.activity_agents.flatMap((entry): UsageActivityAgent[] => {
      if (entry === null || typeof entry !== "object") return [];
      const agent = entry as { id?: unknown; label?: unknown };
      if (typeof agent.id !== "string" || typeof agent.label !== "string") {
        return [];
      }
      return [{ id: agent.id, label: agent.label }];
    })
    : undefined;
  const activityModels = Array.isArray(record.usage?.activity_models)
    ? record.usage.activity_models.flatMap((entry): UsageActivityAgent[] => {
      if (entry === null || typeof entry !== "object") return [];
      const model = entry as { id?: unknown; label?: unknown };
      if (typeof model.id !== "string" || typeof model.label !== "string") {
        return [];
      }
      return [{ id: model.id, label: model.label }];
    })
    : undefined;
  const cacheProtection = parseCacheProtection(record.usage?.cache_protection);
  return {
    ...(account ? { account } : {}),
    ...(reset ? { reset } : {}),
    ...(product ? { product } : {}),
    ...(parser ? { parser } : {}),
    ...(error ? { error } : {}),
    ...(order === undefined ? {} : { order }),
    ...(topBarWindows && topBarWindows.length > 0
      ? { topBarWindows }
      : {}),
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
    const id = (host as { id?: unknown }).id;
    const usage = readUsage(host);
    if (typeof id !== "string" || id === "" || !usage.account) continue;
    maps.plugins[usage.account] = id;
    if (usage.reset) maps.resets[usage.account] = usage.reset;
    if (usage.product) maps.products[usage.account] = usage.product;
    if (usage.parser) maps.parsers[usage.account] = usage.parser;
    if (usage.error) maps.errors[usage.account] = usage.error;
    if (usage.order !== undefined) maps.orders[usage.account] = usage.order;
    if (usage.topBarWindows) maps.windows[usage.account] = usage.topBarWindows;
    if (usage.widget) maps.widgets[usage.account] = usage.widget;
    if (usage.empty) maps.empties[usage.account] = usage.empty;
    if (usage.availableStatus) {
      maps.statuses[usage.account] = usage.availableStatus;
    }
    if (usage.omitEmptyLimits !== undefined) {
      maps.omits[usage.account] = usage.omitEmptyLimits;
    }
    if (usage.widgetShape) maps.shapes[usage.account] = usage.widgetShape;
    if (usage.widgetWindow !== undefined) {
      maps.widgetWindows[usage.account] = usage.widgetWindow;
    }
    if (usage.errorAuth) maps.authCopy[usage.account] = usage.errorAuth;
    if (usage.errorConfig) maps.configCopy[usage.account] = usage.errorConfig;
    if (usage.errorFetch) maps.fetchCopy[usage.account] = usage.errorFetch;
    if (usage.limitIdPrefix) maps.prefixes[usage.account] = usage.limitIdPrefix;
    if (usage.limitLabels) maps.labelRows[usage.account] = usage.limitLabels;
    if (usage.widgetBalanceLabel) {
      maps.balanceLabels[usage.account] = usage.widgetBalanceLabel;
    }
    if (usage.widgetSpendLabel) {
      maps.spendLabels[usage.account] = usage.widgetSpendLabel;
    }
    if (usage.activityAgents) {
      maps.activityAgents[usage.account] = usage.activityAgents;
    }
    if (usage.activityModels) {
      maps.activityModels[usage.account] = usage.activityModels;
    }
    if (usage.cacheProtection) {
      maps.cacheProtection[usage.account] = usage.cacheProtection;
    }
  }
  return maps;
}

const bundledUsage = collectUsageMaps(bundledHostPlugins);
let overlayUsage = emptyUsageMaps();

/** Overlay account→plugin ids declared by activated host plugins. */
export function applyUsageHostPlugins(hosts: unknown): void {
  overlayUsage = collectUsageMaps(hosts);
}

/** Map account-usage provider ids onto Agent Plugin slot ids. */
export function usagePluginId(provider: string): string {
  return overlayUsage.plugins[provider] ?? bundledUsage.plugins[provider] ??
    provider;
}

/** Map account-usage provider ids onto usage-reset API ids. */
export function usageResetId(provider: string): string | undefined {
  return overlayUsage.resets[provider] ?? bundledUsage.resets[provider];
}

/** Display name for an account-usage provider id. */
export function usageProductLabel(provider: string): string {
  return overlayUsage.products[provider] ?? bundledUsage.products[provider] ??
    provider;
}

/** Closed usage-limit parser declared by the account's host plugin. */
export function usageLimitParser(provider: string): string {
  return overlayUsage.parsers[provider] ?? bundledUsage.parsers[provider] ??
    "generic-buckets";
}

/** Closed usage-error presentation declared by the account's host plugin. */
export function usageErrorKind(provider: string): string {
  return overlayUsage.errors[provider] ?? bundledUsage.errors[provider] ??
    "raw";
}

/** First-party card order. Unknown accounts stay insertion-stable. */
export function usageCardOrder(provider: string): number {
  return overlayUsage.orders[provider] ?? bundledUsage.orders[provider] ??
    Number.MAX_SAFE_INTEGER;
}

/** Optional top-bar window minutes declared by the account's host plugin. */
export function usageTopBarWindowMinutes(
  provider: string,
): number[] | undefined {
  const windows = overlayUsage.windows[provider] ??
    bundledUsage.windows[provider];
  return windows && windows.length > 0 ? windows : undefined;
}

/** Closed desktop-widget kind declared by the account's host plugin. */
export function usageWidgetKind(provider: string): string {
  return overlayUsage.widgets[provider] ?? bundledUsage.widgets[provider] ??
    "none";
}

/** Optional empty-state copy declared by the account's host plugin. */
export function usageEmptyMessage(provider: string): string | undefined {
  return overlayUsage.empties[provider] ?? bundledUsage.empties[provider];
}

/** Badge shown when the account is available and has no plan name. */
export function usageAvailableStatus(provider: string): string | undefined {
  return overlayUsage.statuses[provider] ?? bundledUsage.statuses[provider];
}

/** Whether the host should skip the generic empty-limit fallback copy. */
export function usageOmitEmptyLimits(provider: string): boolean {
  if (Object.hasOwn(overlayUsage.omits, provider)) {
    return overlayUsage.omits[provider] === true;
  }
  return bundledUsage.omits[provider] === true;
}

/** Compact desktop-widget shape declared by the account's host plugin. */
export function usageWidgetShape(
  provider: string,
): "percent" | "balance" | "none" {
  const shape = overlayUsage.shapes[provider] ?? bundledUsage.shapes[provider];
  return shape === "percent" || shape === "balance" ? shape : "none";
}

/** Optional compact-widget window minutes declared by the account's host plugin. */
export function usageWidgetWindow(provider: string): number | undefined {
  return overlayUsage.widgetWindows[provider] ??
    bundledUsage.widgetWindows[provider];
}

/** Auth-failure copy declared by the account's host plugin. */
export function usageErrorAuth(provider: string): string | undefined {
  return overlayUsage.authCopy[provider] ?? bundledUsage.authCopy[provider];
}

/** Configuration-failure copy declared by the account's host plugin. */
export function usageErrorConfig(provider: string): string | undefined {
  return overlayUsage.configCopy[provider] ?? bundledUsage.configCopy[provider];
}

/** Fetch-failure copy declared by the account's host plugin. */
export function usageErrorFetch(provider: string): string | undefined {
  return overlayUsage.fetchCopy[provider] ?? bundledUsage.fetchCopy[provider];
}

/** Prefix for parser-owned limit row ids, e.g. claude-five_hour. */
export function usageLimitIdPrefix(provider: string): string {
  return overlayUsage.prefixes[provider] ?? bundledUsage.prefixes[provider] ??
    provider;
}

/** Presentation for one parser-owned rate-limit type. */
export function usageLimitLabel(
  provider: string,
  kind: string,
): { label: string; windowMinutes?: number } {
  const labels = overlayUsage.labelRows[provider] ??
    bundledUsage.labelRows[provider] ??
    [];
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
  return overlayUsage.balanceLabels[provider] ??
    bundledUsage.balanceLabels[provider] ??
    "Balance";
}

/** Compact-widget spend row label declared by the account's host plugin. */
export function usageWidgetSpendLabel(provider: string): string {
  return overlayUsage.spendLabels[provider] ??
    bundledUsage.spendLabels[provider] ??
    "24h spend";
}

function mergedRows<T>(
  overlay: Record<string, T>,
  bundled: Record<string, T>,
): T[] {
  const byAccount = { ...bundled, ...overlay };
  return Object.values(byAccount);
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
    return overlayUsage.activityAgents[provider] ??
      bundledUsage.activityAgents[provider] ??
      [];
  }
  return uniqueAgents(
    mergedRows(overlayUsage.activityAgents, bundledUsage.activityAgents),
  );
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
    return overlayUsage.activityModels[provider] ??
      bundledUsage.activityModels[provider] ??
      [];
  }
  return uniqueAgents(
    mergedRows(overlayUsage.activityModels, bundledUsage.activityModels),
  );
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
    return overlayUsage.cacheProtection[provider] ??
      bundledUsage.cacheProtection[provider];
  }
  return mergedRows(
    overlayUsage.cacheProtection,
    bundledUsage.cacheProtection,
  )[0];
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
