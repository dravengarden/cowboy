import type { DesktopCommand } from "./DesktopCommandProvider";

export interface PaletteCommandEntry {
  readonly command: DesktopCommand;
  readonly available: boolean;
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
 * - Available commands always sort before unavailable ones while preserving
 *   registration order on ties.
 */
export function filterPaletteCommands(
  commands: readonly DesktopCommand[],
  query: string,
): PaletteCommandEntry[] {
  const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const scored: { entry: PaletteCommandEntry; score: number; order: number }[] = [];
  for (let i = 0; i < commands.length; i++) {
    const command = commands[i]!;
    if (command.id === "commandPalette.open") continue;
    const score = scoreCommand(command, tokens);
    if (score < 0) continue;
    const available = command.when?.() !== false;
    scored.push({ entry: { command, available }, score, order: i });
  }
  scored.sort((a, b) => {
    if (a.entry.available !== b.entry.available) {
      return a.entry.available ? -1 : 1;
    }
    if (b.score !== a.score) return b.score - a.score;
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
