import type { SessionMeta } from "./protocol";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

export interface AccountIdentity {
  id?: string;
  label: string;
  short: string;
  description: string;
  named: boolean;
  available: boolean;
}

/** Resolve an immutable binding, never the current default for new sessions.
 * Missing call evidence is unknown; only the Session protocol permits an
 * omitted ID to mean the legacy Service account. Labels confer no authority. */
export function accountIdentity(
  accounts: SubscriptionSnapshot | undefined,
  provider: string,
  id: string | undefined | null,
): AccountIdentity {
  if (!id) return { label: "Not recorded", short: "Account unknown", description: "Execution account was not recorded.", named: false, available: false };
  const named = id !== "default";
  const account = named ? accounts?.registry.accounts.find((item) => item.id === id && item.provider_id === provider) : undefined;
  const label = named ? account?.label : accounts?.display_preferences?.values.legacy_labels[provider];
  const full = label || (named ? "Account unavailable" : "Service account");
  const short = label?.includes(" · ") ? label.slice(label.indexOf(" · ") + 3) : full;
  const available = !named || Boolean(account?.enabled && !account.revoking);
  const status = account?.revoking ? "Access revocation in progress" : account && !account.enabled ? "Disabled" : undefined;
  return {
    id, label: full, short, named, available,
    description: [`Account: ${full}`, status, named && !account ? id : undefined].filter(Boolean).join(" · "),
  };
}

export function sessionAccountIdentity(accounts: SubscriptionSnapshot | undefined, session: Pick<SessionMeta, "provider" | "provider_account_id" | "owner_user_id">): AccountIdentity {
  const scoped = accounts && session.owner_user_id === accounts.dataset.user_id ? accounts : undefined;
  return accountIdentity(scoped, session.provider, session.provider_account_id ?? "default");
}
