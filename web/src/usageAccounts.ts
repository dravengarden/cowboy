import type { SessionMeta } from "./protocol";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import { sessionQuota } from "./sessionQuota";
import type { NativeAccountUsage } from "./nativeAccountUsage";
import { accountControl, accountPlanLabel, namedOnlyProvider } from "./accountControls";
import { accountManageUrl, accountProviderLabel, providerUsage, type ProviderUsage, record, type UsageSnapshot, usageCardProviders, usagePluginId, usageResetProvider, usageResetSchedule } from "./usageLimits";
import type { UsageResetTarget } from "./usageApi";

export interface UsageAccountCard {
  id: string;
  group: string;
  title: string;
  subtitle?: string;
  usage: ProviderUsage;
  /** Service collectors own configurable placement and reset-credit actions. */
  collector?: string;
  emptyMessage?: string;
  statusLabel?: string;
  native?: NativeAccountUsage;
}

/** A session notice must use the same bound identity as Settings and About.
 * Unknown metadata never authorizes falling back to the Service collector. */
export function sessionAccountQuota(
  snapshot: UsageSnapshot | null,
  accounts: SubscriptionSnapshot | undefined,
  sessions: readonly SessionMeta[],
  session: SessionMeta | undefined,
  observations: readonly NativeAccountUsage[] = [],
): ProviderUsage | undefined {
  if (!session) return undefined;
  if (!session.provider_account_id || session.provider_account_id === "default") {
    return providerUsage(snapshot, session.provider, session.provider_version, session.provider_generation_digest);
  }
  if (session.owner_user_id !== accounts?.dataset.user_id) return undefined;
  return registeredUsageAccounts(null, accounts, sessions, observations).find((card) => card.id === session.provider_account_id &&
    accounts?.registry.accounts.some((account) => account.id === card.id && account.provider_id === session.provider))?.usage;
}

/** Registry identity controls membership; Provider IDs only select presentation. */
export function registeredUsageAccounts(
  snapshot: UsageSnapshot | null,
  accounts: SubscriptionSnapshot | undefined,
  sessions: readonly SessionMeta[],
  observations: readonly NativeAccountUsage[] = [],
): UsageAccountCard[] {
  // Revocation is a permanent fence; its cleanup status lives in Subscriptions.
  const named = (accounts?.registry.accounts ?? []).filter((account) => !account.revoking);
  const cards: UsageAccountCard[] = [];
  const added = new Set<string>();
  const namedCard = (account: typeof named[number]): UsageAccountCard => {
    const enabled = account.enabled;
    const reports = enabled ? sessions.filter((session) =>
      session.provider_account_id === account.id && session.provider === account.provider_id &&
      session.owner_user_id === accounts?.dataset.user_id && !session.closing
    ).flatMap((session) => {
      const usage = sessionQuota(session);
      return usage ? [usage] : [];
    }) : [];
    const observed = enabled ? observations.find((item) => item.account_id === account.id && item.provider_id === account.provider_id) : undefined;
    // An account-sandbox (named-only) credential survives Provider upgrades,
    // so an "updated after this account signed in" reason never applies.
    const native: NativeAccountUsage | undefined = observed?.reason === "native_login_outdated" && namedOnlyProvider(accounts, account.provider_id)
      ? withoutReason(observed)
      : observed;
    const collected = native?.usage ?? undefined;
    const usage = collected ?? reports.sort((a, b) => b.observed_at_ms - a.observed_at_ms)[0];
    const provider = usage?.provider ?? snapshot?.providers.find((item) =>
      usagePluginId(item.provider) === account.provider_id
    )?.provider ?? account.provider_id;
    const defaultLabel = accounts?.registry.defaults[account.provider_id] === account.id ? " · Default" : "";
    // A named-only (Antigravity) quota report has no plan; its sessions report one.
    const control = collected && !record(collected.account?.account)?.planType ? accountControl(accounts, account.id) : undefined;
    const plan = control ? accountPlanLabel(control) : undefined;
    return {
      id: account.id, group: provider, title: account.label,
      subtitle: `${accountProviderLabel(provider)}${defaultLabel}`,
      usage: usage ?? { provider, status: "unavailable", source: "", observed_at_ms: 0 },
      ...(native ? { native } : {}),
      ...(plan ? { statusLabel: plan.toUpperCase() } : {}),
      ...(!collected ? { statusLabel: !account.enabled ? "DISABLED" : usage ? "SESSION" : native?.refreshing ? "REFRESHING" : native?.reason === "native_login_outdated" ? "SIGN IN AGAIN" : native?.reason === "native_login_pending" ? "SIGN-IN PENDING" : native?.reason === "native_login_required" || native?.reason === "portable_credentials_required" ? "NOT SIGNED IN" : "NOT REPORTED" } : {}),
      emptyMessage: !account.enabled ? "This account is disabled."
        : native?.message ?? (native?.refreshing ? "Refreshing this account’s usage…" : "No quota reported for this account yet."),
    };
  };
  for (const usage of usageCardProviders(snapshot)) {
    const provider = usagePluginId(usage.provider);
    // A named-only Provider has no Service account to report.
    if (namedOnlyProvider(accounts, provider)) continue;
    const label = accounts?.display_preferences?.values.legacy_labels[provider];
    const group = named.filter((account) => account.provider_id === provider);
    const isDefault = group.length > 0 && (!accounts?.registry.defaults[provider] || accounts.registry.defaults[provider] === "default");
    cards.push({
      id: usage.provider, group: usage.provider, title: label ?? accountProviderLabel(usage.provider),
      ...(label || isDefault ? { subtitle: `${accountProviderLabel(usage.provider)}${isDefault ? " · Default" : ""}` } : {}),
      usage, collector: usage.provider,
    });
    for (const account of group) {
      if (added.has(account.id)) continue;
      cards.push(namedCard(account));
      added.add(account.id);
    }
  }
  // An uninstalled or quota-less Provider must not hide its registered accounts.
  for (const account of named) {
    if (!added.has(account.id)) cards.push(namedCard(account));
  }
  return cards;
}

/** `observation` without its refusal reason and message. */
function withoutReason(observation: NativeAccountUsage): NativeAccountUsage {
  const copy = { ...observation };
  delete copy.reason;
  delete copy.message;
  return copy;
}

/** Old Provider orders keep their accounts together; explicit account order wins. */
export function orderUsageAccounts(cards: UsageAccountCard[], order: string[]): UsageAccountCard[] {
  const rank = new Map(order.map((id, index) => [id, index]));
  const position = (card: UsageAccountCard) => rank.get(card.id) ?? rank.get(card.group) ?? Infinity;
  return [...cards].sort((a, b) => position(a) - position(b));
}

/** Claude reports `max` for both Max and Ultra. When the collector could not
 * read the tier, a reported quota without any 5-hour window is Ultra; this uses
 * only the already-collected usage and never requests anything. */
export function displayPlan(usage: ProviderUsage, plan: string | undefined): string | undefined {
  if (plan !== "max" || usage.provider !== "anthropic") return plan;
  const limits = usage.rate_limits?.rateLimitsByLimitId;
  const windows = limits && typeof limits === "object" ? Object.values(limits) : [];
  if (windows.length === 0) return plan;
  const minutes = (limit: unknown) => (limit as { primary?: { windowDurationMins?: unknown } } | null)?.primary?.windowDurationMins;
  return windows.some((limit) => minutes(limit) === 300) ? plan : "ultra";
}

/** Where a card's reset actions go. A Service card keeps its collector lane.
 * A named card gets its own lane only when the Service can spend its resets
 * and its own collected usage reports reset credits; it never borrows the
 * Service card's lane or credits. */
export function usageResetTarget(card: UsageAccountCard): UsageResetTarget | undefined {
  if (card.collector) {
    const provider = usageResetProvider(card.usage);
    return provider ? { kind: "service", provider } : undefined;
  }
  const collected = card.native?.usage;
  if (!collected || card.native?.resets_supported !== true || usageResetProvider(collected) === undefined) return undefined;
  return record(collected.rate_limits?.rateLimitResetCredits) ? { kind: "account", accountId: card.id } : undefined;
}

/** The scheduled reset shown on this card. Only Service cards schedule;
 * a named card's reset is immediate only. */
export function usageCardResetSchedule(snapshot: UsageSnapshot | null, card: UsageAccountCard): { fire_at_ms: number } | undefined {
  const target = usageResetTarget(card);
  return target?.kind === "service" ? usageResetSchedule(snapshot, card.usage) : undefined;
}

/** The Provider's plan and reset page, on the cards that own reset actions. */
export function usageCardManageUrl(card: UsageAccountCard): string | undefined {
  // The Provider identity decides (named Claude cards included), never
  // whether reset credits happen to be reported right now.
  return accountManageUrl(card.usage.provider);
}
