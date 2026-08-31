const HOUR_MS = 60 * 60 * 1_000;
const DAY_MS = 24 * HOUR_MS;

export const DEFAULT_PASSKEY_REAUTH_INTERVAL_MS = DAY_MS;

export const PASSKEY_REAUTH_INTERVALS = [
  { label: "Every hour", value: HOUR_MS },
  { label: "Every 2 hours", value: 2 * HOUR_MS },
  { label: "Every 3 hours", value: 3 * HOUR_MS },
  { label: "Every 4 hours", value: 4 * HOUR_MS },
  { label: "Every 6 hours", value: 6 * HOUR_MS },
  { label: "Every 12 hours", value: 12 * HOUR_MS },
  { label: "Every day · Default", value: DEFAULT_PASSKEY_REAUTH_INTERVAL_MS },
  { label: "Every 2 days", value: 2 * DAY_MS },
  { label: "Every 3 days", value: 3 * DAY_MS },
] as const;

const RETIRED_EIGHT_HOURS_MS = 8 * HOUR_MS;

export function normalizePasskeyReauthInterval(
  value: number,
  maximum: number,
): number {
  const available = PASSKEY_REAUTH_INTERVALS.filter((option) =>
    option.value <= maximum
  );
  if (available.some((option) => option.value === value)) return value;

  if (value === RETIRED_EIGHT_HOURS_MS) {
    const sixHours = available.find((option) => option.value === 6 * HOUR_MS);
    if (sixHours) return sixHours.value;
  }

  const defaultOption = available.find((option) =>
    option.value === DEFAULT_PASSKEY_REAUTH_INTERVAL_MS
  );
  return defaultOption?.value ?? available.at(-1)?.value ??
    DEFAULT_PASSKEY_REAUTH_INTERVAL_MS;
}
