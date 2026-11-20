// The deliberate order of Provider cards in Settings > Providers and its
// account list: Claude, then Antigravity, then Codex, then the rest. The
// catalog's order (alphabetical by Provider ID) would put Antigravity first
// for Machines and, because Antigravity accounts are named-only (no Service
// credential row), after every other Provider in the account list.

/** Ranked Provider IDs; a Provider not listed follows them in catalog order. */
export const PROVIDER_CARD_ORDER: readonly string[] = Object.freeze([
  "claude-code",
  "claude-deepseek",
  "antigravity",
  "codex",
  "codex-deepseek",
  "grok",
]);

/** A Provider's rank; unlisted Providers share the last one. */
export function providerCardRank(providerId: string): number {
  const rank = PROVIDER_CARD_ORDER.indexOf(providerId);
  return rank < 0 ? PROVIDER_CARD_ORDER.length : rank;
}

/** The rank of a card that shows several Providers (a shared credential). */
function rankOf(providers: readonly string[]): number {
  return Math.min(PROVIDER_CARD_ORDER.length, ...providers.map(providerCardRank));
}

/** `items` stably sorted by the rank of the Providers each one shows. */
export function sortByProviderCardOrder<T>(
  items: readonly T[],
  providers: (item: T) => readonly string[],
): T[] {
  return items
    .map((item, index) => ({ item, index, rank: rankOf(providers(item)) }))
    .sort((left, right) => left.rank - right.rank || left.index - right.index)
    .map(({ item }) => item);
}

/**
 * Where the cards of Providers without a row of their own (named-only
 * Providers such as Antigravity) go among the sorted rows: before the first
 * row ranked after them, keyed by that row's key. Unranked Providers, and
 * ranked ones no row follows, stay `trailing`.
 */
export function namedOnlyPlacement(
  rows: readonly { key: string; providers: readonly string[] }[],
  orphans: readonly string[],
): { before: Map<string, string[]>; trailing: string[] } {
  const before = new Map<string, string[]>();
  const trailing: string[] = [];
  for (const provider of sortByProviderCardOrder(orphans, (id) => [id])) {
    const rank = providerCardRank(provider);
    const row = rank < PROVIDER_CARD_ORDER.length
      ? rows.find((candidate) => rankOf(candidate.providers) > rank)
      : undefined;
    if (row) before.set(row.key, [...(before.get(row.key) ?? []), provider]);
    else trailing.push(provider);
  }
  return { before, trailing };
}
