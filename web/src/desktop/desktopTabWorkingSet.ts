// Working-set partitioning for the Desktop top tab strip: keeps at most
// `MAX_VISIBLE_DESKTOP_TABS` (9) tabs directly on the bar so every visible
// tab maps 1-to-1 to `␣1..9`, titles remain legible without scrolling, and
// extra open tabs fold cleanly into the overflow menu without losing state.

export const MAX_VISIBLE_DESKTOP_TABS = 9;

export interface WorkingSetTabLike {
  readonly key: string;
  readonly pinned: boolean;
}

export interface DesktopTabWorkingSet<T extends WorkingSetTabLike> {
  readonly visible: readonly T[];
  readonly overflow: readonly T[];
}

export function partitionDesktopTabWorkingSet<T extends WorkingSetTabLike>(
  items: readonly T[],
  currentKey: string | null,
  visits: readonly string[],
  limit = MAX_VISIBLE_DESKTOP_TABS,
): DesktopTabWorkingSet<T> {
  if (items.length <= limit) return { visible: items, overflow: [] };
  const chosen = new Set<string>();
  const byKey = new Map(items.map((item) => [item.key, item]));
  // Always keep the active tab in the visible working set first, even when
  // `limit` or more tabs are pinned, so the current tab retains its active
  // pill and leader slot and can never land in bulk overflow cleanup.
  if (limit > 0 && currentKey && byKey.has(currentKey)) {
    chosen.add(currentKey);
  }
  const pinned = items.filter((item) => item.pinned);
  for (const item of pinned) {
    if (chosen.size >= limit) break;
    chosen.add(item.key);
  }
  for (const key of visits) {
    if (chosen.size >= limit) break;
    if (byKey.has(key)) chosen.add(key);
  }
  // Fill any remaining slots with neighbours around the current tab so the
  // working strip stays anchored near the user's current position.
  if (chosen.size < limit) {
    const anchor = currentKey ? items.findIndex((item) => item.key === currentKey) : 0;
    const base = anchor === -1 ? 0 : anchor;
    for (let dist = 1; dist < items.length && chosen.size < limit; dist++) {
      for (const idx of [base - dist, base + dist]) {
        const candidate = items[idx];
        if (candidate && !chosen.has(candidate.key)) {
          chosen.add(candidate.key);
          if (chosen.size >= limit) break;
        }
      }
    }
  }
  const visible = items.filter((item) => chosen.has(item.key));
  const visitRank = new Map(visits.map((key, idx) => [key, idx]));
  const overflow = items
    .filter((item) => !chosen.has(item.key))
    .sort((a, b) => (visitRank.get(a.key) ?? 1e9) - (visitRank.get(b.key) ?? 1e9));
  return { visible, overflow };
}
