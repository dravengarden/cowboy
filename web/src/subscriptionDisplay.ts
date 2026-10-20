export interface SubscriptionDisplay {
  scope: "subscriptions";
  revision: number;
  values: { legacy_labels: Record<string, string> };
}

export function validSubscriptionLabel(value: string): boolean {
  return Boolean(value.trim()) && new TextEncoder().encode(value).length <= 128 &&
    !Array.from(value).some((character) => {
      const code = character.codePointAt(0)!;
      return code < 32 || (code >= 127 && code <= 159);
    });
}

export function validLegacyLabels(value: unknown): value is Record<string, string> {
  return typeof value === "object" && value !== null && !Array.isArray(value) &&
    Object.keys(value).length <= 128 && Object.entries(value).every(([provider, label]) =>
      /^[a-z0-9-]{1,64}$/.test(provider) && typeof label === "string" &&
      label === label.trim() && validSubscriptionLabel(label)
    );
}

export function validSubscriptionDisplay(value: unknown): value is SubscriptionDisplay {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const display = value as Partial<SubscriptionDisplay>;
  return display.scope === "subscriptions" && typeof display.revision === "number" &&
    Number.isSafeInteger(display.revision) && display.revision >= 0 && validLegacyLabels(display.values?.legacy_labels);
}

export function legacySubscriptionLabel(display: SubscriptionDisplay | undefined, provider: string): string {
  const labels = display?.values.legacy_labels;
  return labels && Object.hasOwn(labels, provider) ? labels[provider]! : "Existing account";
}
