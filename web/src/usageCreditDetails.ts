/** Read-only display rows from a provider's optional credit projection. */
export interface UsageCreditRow {
  label: string;
  value: string;
}

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

export function creditMoney(
  value: unknown,
  currency: unknown,
): string | undefined {
  if (
    typeof value !== "number" || !Number.isFinite(value) || value < 0 ||
    typeof currency !== "string" || !/^[A-Z]{3}$/.test(currency)
  ) return undefined;
  const formatter = new Intl.NumberFormat(undefined, {
    style: "currency",
    currency,
  });
  const digits = formatter.resolvedOptions().maximumFractionDigits ?? 2;
  return formatter.format(value / 10 ** digits);
}

export function usageCreditDetails(rateLimits: unknown): UsageCreditRow[] {
  const limits = record(rateLimits);
  const credit = record(limits.usageCredits);
  const rows: UsageCreditRow[] = [];
  const addMoney = (label: string, value: unknown, currency: unknown) => {
    const formatted = creditMoney(value, currency);
    if (formatted !== undefined) rows.push({ label, value: formatted });
    else if (
      typeof value === "number" && Number.isFinite(value) && value >= 0
    ) {
      rows.push({
        label,
        value: `${value.toLocaleString()} minor units · currency not reported`,
      });
    }
  };
  if (typeof credit.enabled === "boolean") {
    rows.push({ label: "Usage credits", value: credit.enabled ? "On" : "Off" });
  }
  addMoney("Balance", credit.balanceMinor, credit.balanceCurrency);
  if (credit.balanceStatus === "unavailable") {
    rows.push({ label: "Balance", value: "Unavailable" });
  }
  addMoney("Spent this month", credit.spentMinor, credit.spendCurrency);
  addMoney(
    "Monthly spending limit",
    credit.monthlyLimitMinor,
    credit.spendCurrency,
  );
  if (typeof credit.autoReloadEnabled === "boolean") {
    rows.push({
      label: "Auto reload",
      value: credit.autoReloadEnabled ? "On" : "Off",
    });
  }
  if (
    typeof credit.expiryPolicyMonths === "number" &&
    Number.isSafeInteger(credit.expiryPolicyMonths) &&
    credit.expiryPolicyMonths > 0
  ) {
    rows.push({
      label: "Credit expiry policy",
      value: `${credit.expiryPolicyMonths} months`,
    });
  }
  const resets = record(limits.rateLimitResetCredits);
  if (
    resets.availableCount === 0 &&
    (!Array.isArray(resets.grants) || resets.grants.length === 0)
  ) {
    rows.push({ label: "Limit resets", value: "None available" });
  }
  if (Array.isArray(resets.grants)) {
    for (const entry of resets.grants.slice(0, 128)) {
      const grant = record(entry);
      if (
        typeof grant.title !== "string" ||
        typeof grant.remaining !== "number" ||
        !Number.isSafeInteger(grant.remaining) || grant.remaining <= 0
      ) continue;
      const status = grant.status === "available"
        ? "available"
        : grant.status === "paused"
        ? "paused"
        : "not currently usable";
      const date = typeof grant.expiresAt === "number"
        ? new Date(grant.expiresAt * 1000)
        : undefined;
      const expiry = date && Number.isFinite(date.getTime())
        ? ` · expires ${date.toLocaleString()}`
        : "";
      rows.push({
        label: `${grant.title} resets`,
        value: `${grant.remaining} ${status}${expiry}`,
      });
    }
  }
  return rows;
}
