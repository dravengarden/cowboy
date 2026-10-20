import type { SessionMeta } from "./protocol";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import { sessionQuota } from "./sessionQuota";
import { accountProviderLabel, providerUsage, type ProviderUsage, type UsageSnapshot, usageCardProviders, usagePluginId } from "./usageLimits";

export interface UsageAccountCard {
  id: string;
  group: string;
  title: string;
  subtitle?: string;
  usage: ProviderUsage;
  /** Only Service collectors currently support refresh, placement and resets. */
  collector?: string;
  emptyMessage?: string;
  statusLabel?: string;
}

/** A session notice must use the same bound identity as Settings and About.
 * Unknown metadata never authorizes falling back to the Service collector. */
export function sessionAccountQuota(
  snapshot: UsageSnapshot | null,
  accounts: SubscriptionSnapshot | undefined,
  sessions: readonly SessionMeta[],
  session: SessionMeta | undefined,
): ProviderUsage | undefined {
  if (!session) return undefined;
  if (!session.provider_account_id || session.provider_account_id === "default") {
    return providerUsage(snapshot, session.provider, session.provider_version, session.provider_generation_digest);
  }
  if (session.owner_user_id !== accounts?.dataset.user_id) return undefined;
  return registeredUsageAccounts(null, accounts, sessions).find((card) => card.id === session.provider_account_id &&
    accounts?.registry.accounts.some((account) => account.id === card.id && account.provider_id === session.provider))?.usage;
}

/** Registry identity controls membership; Provider IDs only select presentation. */
export function registeredUsageAccounts(
  snapshot: UsageSnapshot | null,
  accounts: SubscriptionSnapshot | undefined,
  sessions: readonly SessionMeta[],
): UsageAccountCard[] {
  const named = accounts?.registry.accounts ?? [];
  const cards: UsageAccountCard[] = [];
  const added = new Set<string>();
  const namedCard = (account: typeof named[number]): UsageAccountCard => {
    const enabled = account.enabled && !account.revoking;
    const reports = enabled ? sessions.filter((session) =>
      session.provider_account_id === account.id && session.provider === account.provider_id &&
      session.owner_user_id === accounts?.dataset.user_id && !session.closing
    ).flatMap((session) => {
      const usage = sessionQuota(session);
      return usage ? [usage] : [];
    }) : [];
    const usage = reports.sort((a, b) => b.observed_at_ms - a.observed_at_ms)[0];
    const provider = usage?.provider ?? snapshot?.providers.find((item) =>
      usagePluginId(item.provider) === account.provider_id
    )?.provider ?? account.provider_id;
    const defaultLabel = accounts?.registry.defaults[account.provider_id] === account.id ? " · Default" : "";
    return {
      id: account.id, group: provider, title: account.label,
      subtitle: `${accountProviderLabel(provider)}${defaultLabel}`,
      usage: usage ?? { provider, status: "unavailable", source: "", observed_at_ms: 0 },
      statusLabel: account.revoking ? "REVOKING" : !account.enabled ? "DISABLED" : usage ? "SESSION" : "NOT REPORTED",
      emptyMessage: account.revoking ? "Access revocation is in progress."
        : !account.enabled ? "This account is disabled."
        : "No quota reported for this account yet. Background quota refresh is not available; quota reported by this account’s sessions appears here.",
    };
  };
  for (const usage of usageCardProviders(snapshot)) {
    const provider = usagePluginId(usage.provider);
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

/** Old Provider orders keep their accounts together; explicit account order wins. */
export function orderUsageAccounts(cards: UsageAccountCard[], order: string[]): UsageAccountCard[] {
  const rank = new Map(order.map((id, index) => [id, index]));
  const position = (card: UsageAccountCard) => rank.get(card.id) ?? rank.get(card.group) ?? Infinity;
  return [...cards].sort((a, b) => position(a) - position(b));
}
