import type { SessionMeta } from "./protocol";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import { sessionQuota } from "./sessionQuota";
import type { NativeAccountUsage } from "./nativeAccountUsage";
import { accountControl, accountPlanLabel, namedOnlyProvider } from "./accountControls";
import { accountManageUrl, providerManageUrl, accountProviderLabel, nearestAvailableResetCredit, type ProviderUsage, record, type UsageSnapshot, usageAccountId, usageCardOrder, usagePluginId, usageResetProvider } from "./usageLimits";
import type { UsageResetTarget } from "./usageApi";

export interface UsageAccountCard {
  id: string;
  group: string;
  title: string;
  subtitle?: string;
  usage: ProviderUsage;
  emptyMessage?: string;
  statusLabel?: string;
  native?: NativeAccountUsage;
  /** A named card's registered Provider (`claude-code`). */
  providerId?: string;
}

export function sameSessionQuotaInputs(
  previous: readonly SessionMeta[],
  next: readonly SessionMeta[],
): boolean {
  if (previous === next) return true;
  let pi = 0;
  let ni = 0;
  const isQuotaInput = (s: SessionMeta): boolean =>
    Boolean(s.provider_account_id && s.provider_account_id !== "default" && !s.closing && sessionQuota(s) !== undefined);
  while (true) {
    while (pi < previous.length && !isQuotaInput(previous[pi]!)) pi += 1;
    while (ni < next.length && !isQuotaInput(next[ni]!)) ni += 1;
    if (pi >= previous.length || ni >= next.length) {
      return pi >= previous.length && ni >= next.length;
    }
    const a = previous[pi]!;
    const b = next[ni]!;
    if (
      a !== b && (
        a.id !== b.id ||
        a.provider !== b.provider ||
        a.provider_account_id !== b.provider_account_id ||
        a.owner_user_id !== b.owner_user_id ||
        sessionQuota(a) !== sessionQuota(b)
      )
    ) return false;
    pi += 1;
    ni += 1;
  }
}

/** A session notice must use the same bound identity as Settings and About.
 * A retired legacy default account has no quota card at all. */
export function sessionAccountQuota(
  _snapshot: UsageSnapshot | null,
  accounts: SubscriptionSnapshot | undefined,
  sessions: readonly SessionMeta[],
  session: SessionMeta | undefined,
  observations: readonly NativeAccountUsage[] = [],
): ProviderUsage | undefined {
  if (!session) return undefined;
  if (!session.provider_account_id || session.provider_account_id === "default") return undefined;
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
    const provider = usage?.provider ?? usageAccountId(account.provider_id) ??
      snapshot?.providers.find((item) =>
        usagePluginId(item.provider) === account.provider_id
      )?.provider ?? account.provider_id;
    const defaultLabel = accounts?.registry.defaults[account.provider_id] === account.id ? " · Default" : "";
    // A named-only (Antigravity) quota report has no plan; its sessions report one.
    const control = collected && !record(collected.account?.account)?.planType ? accountControl(accounts, account.id) : undefined;
    const plan = control ? accountPlanLabel(control) : undefined;
    return {
      id: account.id, group: provider, title: account.label, providerId: account.provider_id,
      subtitle: `${accountProviderLabel(provider)}${defaultLabel}`,
      usage: usage ?? { provider, status: "unavailable", source: "", observed_at_ms: 0 },
      ...(native ? { native } : {}),
      ...(plan ? { statusLabel: plan.toUpperCase() } : {}),
      ...(!collected ? { statusLabel: !account.enabled ? "DISABLED" : usage ? "SESSION" : native?.refreshing ? "REFRESHING" : native?.reason === "native_login_outdated" || native?.reason === "native_usage_authentication_required" || native?.reason === "portable_usage_authentication_required" ? "SIGN IN AGAIN" : native?.reason === "native_login_pending" ? "SIGN-IN PENDING" : native?.reason === "native_login_required" || native?.reason === "portable_credentials_required" ? "NOT SIGNED IN" : "NOT REPORTED" } : {}),
      emptyMessage: !account.enabled ? "This account is disabled."
        : native?.message ?? (native?.refreshing ? "Refreshing this account’s usage…" : "No quota reported for this account yet."),
    };
  };
  // Provider product order first (the legacy default cards are retired, so
  // every card is a registered account); accounts without a declared order
  // keep their registry order behind them.
  return named
    .map((account, index) => ({ card: namedCard(account), index }))
    .sort((left, right) =>
      usageCardOrder(left.card.group) - usageCardOrder(right.card.group) ||
      left.index - right.index
    )
    .map(({ card }) => card);
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

/** Where a card's reset actions go: the account's own lane, only when the
 * Service can spend its resets and its own collected usage reports reset
 * credits. */
export function usageResetTarget(card: UsageAccountCard): UsageResetTarget | undefined {
  const collected = card.native?.usage;
  if (!collected || card.native?.resets_supported !== true || usageResetProvider(collected) === undefined) return undefined;
  return record(collected.rate_limits?.rateLimitResetCredits) ? { kind: "account", accountId: card.id } : undefined;
}

/** The owner's pending schedule of this card's account reset. */
export function usageCardResetSchedule(card: UsageAccountCard): { fire_at_ms: number } | undefined {
  const schedule = card.native?.reset_schedule;
  return typeof schedule?.fire_at_ms === "number" ? schedule : undefined;
}

/** Why a card whose own usage reports an available reset cannot spend it,
 * so the card says so instead of only hiding the action. */
export function usageResetUnavailableMessage(card: UsageAccountCard): string | undefined {
  const native = card.native;
  if (!native?.usage || native.resets_supported === true) return undefined;
  if (nearestAvailableResetCredit(native.usage) === undefined) return undefined;
  return native.reset_unavailable_message;
}

/** The Provider's plan and reset page, on the cards that own reset actions. */
export function usageCardManageUrl(card: UsageAccountCard): string | undefined {
  // The Provider identity decides (named Claude cards included, even with no
  // observation or snapshot), never whether reset credits are reported now.
  return accountManageUrl(card.usage.provider) ??
    (card.providerId === undefined ? undefined : providerManageUrl(card.providerId));
}
