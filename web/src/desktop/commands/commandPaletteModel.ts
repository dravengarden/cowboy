import type { DesktopCommand } from "./DesktopCommandProvider";

export interface PaletteCommandEntry {
  readonly command: DesktopCommand;
  readonly available: boolean;
  readonly recent: boolean;
}

export const PALETTE_RECENT_LIMIT = 5;
const PALETTE_RECENT_STORAGE_KEY = "cowboy:desktop-palette-recent-v1";

/** Return an updated MRU list with `id` moved to the front, capped at `limit`. */
export function recordRecentPaletteCommand(
  recentIds: readonly string[],
  id: string,
  limit = PALETTE_RECENT_LIMIT,
): string[] {
  if (!id || id === "commandPalette.open") return [...recentIds].slice(0, limit);
  const next = [id, ...recentIds.filter((item) => item !== id)];
  return next.slice(0, Math.max(0, limit));
}

export function loadRecentPaletteCommands(
  storage: Pick<Storage, "getItem"> | null | undefined = globalThis.localStorage,
): string[] {
  try {
    const raw = storage?.getItem(PALETTE_RECENT_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((item): item is string => typeof item === "string" && item.length > 0)
      .slice(0, PALETTE_RECENT_LIMIT);
  } catch {
    return [];
  }
}

export function saveRecentPaletteCommands(
  recentIds: readonly string[],
  storage: Pick<Storage, "setItem"> | null | undefined = globalThis.localStorage,
): void {
  try {
    storage?.setItem(
      PALETTE_RECENT_STORAGE_KEY,
      JSON.stringify(recentIds.slice(0, PALETTE_RECENT_LIMIT)),
    );
  } catch {
    // Storage quota or private-mode restriction must never block command execution.
  }
}

function tokenMatchScore(haystack: string, title: string, token: string): number {
  const titleIndex = title.indexOf(token);
  if (titleIndex === 0) return 120;
  if (titleIndex > 0) {
    const boundary = /[\s:./_-]/.test(title[titleIndex - 1] ?? "");
    return boundary ? 95 : 70;
  }
  const haystackIndex = haystack.indexOf(token);
  if (haystackIndex >= 0) return 45;

  // Ordered subsequence fallback on title so abbreviations like "rvdf" or
  // "clsp" still find "Review: Unified or split diff" / "Collapse Prompt".
  let cursor = 0;
  let gapPenalty = 0;
  for (let i = 0; i < token.length; i++) {
    const found = title.indexOf(token[i]!, cursor);
    if (found < 0) return -1;
    gapPenalty += found - cursor;
    cursor = found + 1;
  }
  return Math.max(5, 30 - Math.min(25, gapPenalty));
}

function scoreCommand(command: DesktopCommand, tokens: readonly string[]): number {
  if (tokens.length === 0) return 0;
  const title = command.title.toLowerCase();
  const haystack = `${title} ${command.group.toLowerCase()} ${(command.description ?? "").toLowerCase()} ${command.id.toLowerCase()}`;
  let total = 0;
  for (const token of tokens) {
    const score = tokenMatchScore(haystack, title, token);
    if (score < 0) return -1;
    total += score;
  }
  return total;
}

/**
 * Filter and rank Command Palette entries:
 * - Excludes `commandPalette.open`.
 * - Multi-token substring + title subsequence matching.
 * - Available commands always sort before unavailable ones.
 * - Recently executed commands (`recentIds`) float to the top when browsing
 *   and break match-score ties while searching.
 */
export function filterPaletteCommands(
  commands: readonly DesktopCommand[],
  query: string,
  recentIds: readonly string[] = [],
): PaletteCommandEntry[] {
  const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const recentRank = new Map<string, number>();
  recentIds.forEach((id, idx) => {
    if (!recentRank.has(id)) recentRank.set(id, idx);
  });
  const scored: {
    entry: PaletteCommandEntry;
    score: number;
    recentOrder: number;
    order: number;
  }[] = [];
  for (let i = 0; i < commands.length; i++) {
    const command = commands[i]!;
    if (command.id === "commandPalette.open") continue;
    const score = scoreCommand(command, tokens);
    if (score < 0) continue;
    const available = command.when?.() !== false;
    const rank = recentRank.get(command.id);
    scored.push({
      entry: { command, available, recent: rank !== undefined },
      score,
      recentOrder: rank ?? Number.POSITIVE_INFINITY,
      order: i,
    });
  }
  scored.sort((a, b) => {
    if (a.entry.available !== b.entry.available) {
      return a.entry.available ? -1 : 1;
    }
    if (b.score !== a.score) return b.score - a.score;
    if (a.recentOrder !== b.recentOrder) return a.recentOrder - b.recentOrder;
    return a.order - b.order;
  });
  return scored.map((item) => item.entry);
}

/**
 * Step the Command Palette selection in `direction` (`1` or `-1`), skipping
 * unavailable entries when at least one command is available. Clamps at the
 * ends of the available run so navigation stays predictable.
 */
export function stepPaletteSelection(
  entries: readonly PaletteCommandEntry[],
  current: number,
  direction: 1 | -1,
): number {
  if (entries.length === 0) return 0;
  const hasAvailable = entries.some((entry) => entry.available);
  if (!hasAvailable) {
    return Math.max(0, Math.min(entries.length - 1, current + direction));
  }
  for (
    let next = current + direction;
    next >= 0 && next < entries.length;
    next += direction
  ) {
    if (entries[next]!.available) return next;
  }
  if (entries[current]?.available) return current;
  return firstSelectablePaletteIndex(entries);
}

/** First available entry index, or `0` when all entries are unavailable/empty. */
export function firstSelectablePaletteIndex(
  entries: readonly PaletteCommandEntry[],
): number {
  const index = entries.findIndex((entry) => entry.available);
  return index >= 0 ? index : 0;
}
