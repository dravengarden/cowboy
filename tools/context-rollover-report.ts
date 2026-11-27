// Read-only report: does context renewal (docs/context-rollover.md) save what
// it should? Joins each turn's persisted usage with the idle gap before its
// opening prompt and whether a `context_rolled_over` marker preceded it, so a
// renewed first turn can be compared with ordinary cold resumes.
//
//   bun tools/context-rollover-report.ts /var/lib/cowboy/cowboy.sqlite3 [--days 30] [--json]
import { Database } from "bun:sqlite";

export interface RawTurnRow {
  provider: string;
  input: number;
  cached: number | null;
  written: number | null;
  uncached: number | null;
  gap_ms: number | null;
  renewed: number;
}

/**
 * Disjoint cache token breakdown for one turn.
 *
 * `TurnUsage.uncached_input_tokens` equals `input_tokens - cached_input_tokens`
 * and therefore already includes `cache_write_tokens` (e.g. Claude Messages).
 * Never sum `cache_write_tokens + uncached_input_tokens`.
 */
export interface NormalizedTurnCacheUsage {
  inclusive_input: number;
  cached_read: number | null;
  cache_write: number | null;
  uncached_total: number | null;
  other_uncached: number | null;
}

function parseOptionalSafeNonNegativeInt(
  value: number | null | undefined,
): { ok: true; value: number | null } | { ok: false } {
  if (value === null || value === undefined) {
    return { ok: true, value: null };
  }
  if (!Number.isSafeInteger(value) || value < 0) {
    return { ok: false };
  }
  return { ok: true, value };
}

export function normalizeTurnCacheUsage(row: {
  input: number;
  cached: number | null;
  written: number | null;
  uncached: number | null;
}): NormalizedTurnCacheUsage | null {
  if (!Number.isSafeInteger(row.input) || row.input < 0) return null;
  const inclusive_input = row.input;

  const parsedCached = parseOptionalSafeNonNegativeInt(row.cached);
  const parsedWritten = parseOptionalSafeNonNegativeInt(row.written);
  const parsedUncached = parseOptionalSafeNonNegativeInt(row.uncached);
  if (!parsedCached.ok || !parsedWritten.ok || !parsedUncached.ok) return null;

  const cached_read = parsedCached.value;
  const cache_write = parsedWritten.value;
  const raw_uncached = parsedUncached.value;

  if (cached_read !== null && cached_read > inclusive_input) return null;
  if (cache_write !== null && cache_write > inclusive_input) return null;
  if (raw_uncached !== null && raw_uncached > inclusive_input) return null;

  // `uncached_total` is strictly `inclusive_input - cached_read` when observed.
  const derived_uncached =
    cached_read !== null ? inclusive_input - cached_read : raw_uncached;
  if (
    derived_uncached !== null &&
    raw_uncached !== null &&
    derived_uncached !== raw_uncached
  ) {
    return null;
  }
  const uncached_total = derived_uncached;

  if (
    cache_write !== null &&
    uncached_total !== null &&
    cache_write > uncached_total
  ) {
    return null;
  }
  const other_uncached =
    uncached_total !== null && cache_write !== null
      ? uncached_total - cache_write
      : null;

  return {
    inclusive_input,
    cached_read,
    cache_write,
    uncached_total,
    other_uncached,
  };
}

export const GAP_BUCKETS: [string, number][] = [
  ["<5m", 5 * 60_000],
  ["5-30m", 30 * 60_000],
  ["30-60m", 60 * 60_000],
  ["1-3h", 3 * 3_600_000],
  ["3-12h", 12 * 3_600_000],
  [">12h", Number.POSITIVE_INFINITY],
];

const bucket = (gap: number): string =>
  GAP_BUCKETS.find(([, limit]) => gap < limit)![0];

export interface RolloverReportRow {
  provider: string;
  gap: string;
  renewed: boolean;
  turns: number;
  observed_cache_turns: number;
  mean_input: number;
  mean_cached_read: number | null;
  mean_cache_write: number | null;
  mean_uncached_total: number | null;
  cached_share: number | null;
  uncached_share: number | null;
  /** Compatibility alias for `uncached_share` (docs/context-rollover.md). */
  fresh_ratio: number | null;
}

interface Group {
  provider: string;
  gap: string;
  renewed: boolean;
  turns: number;
  input: number;
  observed_cache_turns: number;
  observed_cache_input: number;
  cached_read: number;
  uncached_total: number;
  observed_write_turns: number;
  cache_write: number;
}

export function aggregateRolloverReport(
  rows: readonly RawTurnRow[],
): RolloverReportRow[] {
  const groups = new Map<string, Group>();
  for (const row of rows) {
    if (row.gap_ms === null || row.gap_ms < 0) continue;
    const normalized = normalizeTurnCacheUsage(row);
    if (!normalized) continue;
    const gap = bucket(row.gap_ms);
    const key = `${row.provider}|${gap}|${row.renewed}`;
    const group = groups.get(key) ?? {
      provider: row.provider,
      gap,
      renewed: row.renewed === 1,
      turns: 0,
      input: 0,
      observed_cache_turns: 0,
      observed_cache_input: 0,
      cached_read: 0,
      uncached_total: 0,
      observed_write_turns: 0,
      cache_write: 0,
    };
    group.turns += 1;
    group.input += normalized.inclusive_input;
    if (normalized.cached_read !== null && normalized.uncached_total !== null) {
      group.observed_cache_turns += 1;
      group.observed_cache_input += normalized.inclusive_input;
      group.cached_read += normalized.cached_read;
      group.uncached_total += normalized.uncached_total;
      if (normalized.cache_write !== null) {
        group.observed_write_turns += 1;
        group.cache_write += normalized.cache_write;
      }
    }
    groups.set(key, group);
  }
  return [...groups.values()]
    .sort(
      (a, b) =>
        a.provider.localeCompare(b.provider) ||
        GAP_BUCKETS.findIndex(([name]) => name === a.gap) -
          GAP_BUCKETS.findIndex(([name]) => name === b.gap) ||
        Number(a.renewed) - Number(b.renewed),
    )
    .map((group) => {
      const uncached_share =
        group.observed_cache_input > 0
          ? Number(
              (group.uncached_total / group.observed_cache_input).toFixed(4),
            )
          : null;
      return {
        provider: group.provider,
        gap: group.gap,
        renewed: group.renewed,
        turns: group.turns,
        observed_cache_turns: group.observed_cache_turns,
        mean_input: Math.round(group.input / group.turns),
        mean_cached_read:
          group.observed_cache_turns > 0
            ? Math.round(group.cached_read / group.observed_cache_turns)
            : null,
        mean_cache_write:
          group.observed_write_turns > 0
            ? Math.round(group.cache_write / group.observed_write_turns)
            : null,
        mean_uncached_total:
          group.observed_cache_turns > 0
            ? Math.round(group.uncached_total / group.observed_cache_turns)
            : null,
        cached_share:
          group.observed_cache_input > 0
            ? Number((group.cached_read / group.observed_cache_input).toFixed(4))
            : null,
        uncached_share,
        fresh_ratio: uncached_share,
      };
    });
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const path = args.find((arg) => !arg.startsWith("--"));
  if (!path) {
    console.error(
      "usage: context-rollover-report.ts <cowboy.sqlite3> [--days N] [--json]",
    );
    process.exit(2);
  }
  const daysIndex = args.indexOf("--days");
  const days = daysIndex >= 0 ? Number(args[daysIndex + 1]) : 30;
  const since = Date.now() - days * 86_400_000;

  const db = new Database(path, { readonly: true });
  db.exec("PRAGMA query_only = 1");

  // One row per completed turn whose FIRST non-steering prompt opener is a
  // human prompt (never matching a later human steer on an automated turn).
  const rows = db
    .query(`
  WITH te AS (
    SELECT e.session_id, e.seq, e.ts_ms, s.provider,
      json_extract(e.payload, '$.usage.input_tokens') AS input,
      json_extract(e.payload, '$.usage.cached_input_tokens') AS cached,
      json_extract(e.payload, '$.usage.cache_write_tokens') AS written,
      json_extract(e.payload, '$.usage.uncached_input_tokens') AS uncached,
      lag(e.ts_ms) OVER (PARTITION BY e.session_id ORDER BY e.seq) AS prev_end,
      lag(e.seq) OVER (PARTITION BY e.session_id ORDER BY e.seq) AS prev_seq
    FROM events e JOIN sessions s ON s.id = e.session_id
    WHERE json_extract(e.payload, '$.kind') = 'turn_end' AND e.ts_ms >= ?1
  ),
  opener AS (
    SELECT te.*,
      (SELECT x.seq FROM events x WHERE x.session_id = te.session_id
         AND x.seq > te.prev_seq AND x.seq < te.seq
         AND json_extract(x.payload, '$.kind') = 'update'
         AND json_extract(x.payload, '$.update.sessionUpdate') = 'user_message_chunk'
         AND COALESCE(json_extract(x.payload, '$.update._meta.cowboyDelivery'), '') != 'current_turn'
       ORDER BY x.seq ASC LIMIT 1) AS opener_seq
    FROM te
    WHERE te.prev_end IS NOT NULL AND te.input > 0
  )
  SELECT o.provider, o.input, o.cached, o.written, o.uncached,
    CASE
      WHEN COALESCE(json_extract(p.payload, '$.update.promptOrigin.actor'), 'human') = 'human'
       AND COALESCE(json_extract(p.payload, '$.update.autoResumed'), 0) = 0
       AND COALESCE(json_extract(p.payload, '$.update.content.text'), '') NOT LIKE '<system-reminder>%'
      THEN p.ts_ms - o.prev_end
      ELSE NULL
    END AS gap_ms,
    EXISTS (SELECT 1 FROM events x WHERE x.session_id = o.session_id
       AND x.seq > o.prev_seq AND x.seq < o.seq
       AND json_extract(x.payload, '$.update.sessionUpdate') = 'context_rolled_over') AS renewed
  FROM opener o
  JOIN events p ON p.session_id = o.session_id AND p.seq = o.opener_seq
`)
    .all(since) as RawTurnRow[];

  const report = aggregateRolloverReport(rows);

  if (args.includes("--json")) {
    console.log(JSON.stringify({ since_ms: since, days, groups: report }, null, 2));
  } else {
    console.log(`Turns since ${new Date(since).toISOString()} (${days} days)`);
    console.table(report);
  }
}
