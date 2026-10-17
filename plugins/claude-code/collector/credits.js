// Read-only subscription credit projections. Never derive a balance from spend.
const currencyCode = (value) =>
  typeof value === "string" && /^[A-Z]{3}$/.test(value) ? value : undefined;
const amount = (value) =>
  typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : undefined;

export function spendCredits(value) {
  if (!value || typeof value.is_enabled !== "boolean") return undefined;
  const currency = currencyCode(value.currency);
  const used = amount(value.used_credits);
  const limit = amount(value.monthly_limit);
  return {
    enabled: value.is_enabled,
    ...(currency ? { spendCurrency: currency } : {}),
    ...(used === undefined ? {} : { spentMinor: used }),
    ...(limit === undefined ? {} : { monthlyLimitMinor: limit }),
  };
}

export function prepaidCredits(value) {
  const balance = amount(value?.amount);
  const currency = currencyCode(value?.currency);
  if (balance === undefined || !currency) {
    throw new Error("Claude credit balance is unavailable.");
  }
  const enabled = value.auto_reload_settings?.enabled;
  const expiry = value.expiry_policy_months;
  return {
    balanceMinor: balance,
    balanceCurrency: currency,
    ...(typeof enabled === "boolean" ? { autoReloadEnabled: enabled } : {}),
    ...(Number.isSafeInteger(expiry) && expiry > 0
      ? { expiryPolicyMonths: expiry }
      : {}),
  };
}

export async function fetchPrepaidCredits(client) {
  const profile = await client("/api/oauth/profile");
  const org = profile.organization?.uuid;
  if (typeof org !== "string" || !/^[0-9a-f-]{36}$/.test(org)) {
    throw new Error("Claude credit balance is unavailable.");
  }
  return prepaidCredits(
    await client(`/api/oauth/organizations/${org}/prepaid/credits`),
  );
}
