// Read-only report: does context renewal (docs/context-rollover.md) save what
// it should? Joins each turn's persisted usage with the idle gap before its
// human prompt and whether a `context_rolled_over` marker preceded it, so a
// renewed first turn can be compared with ordinary cold resumes.
//
//   bun tools/context-rollover-report.ts /var/lib/cowboy/cowboy.sqlite3 [--days 30] [--json]
import { Database } from "bun:sqlite";

const args = process.argv.slice(2);
const path = args.find((arg) => !arg.startsWith("--"));
if (!path) {
  console.error("usage: context-rollover-report.ts <cowboy.sqlite3> [--days N] [--json]");
  process.exit(2);
}
const daysIndex = args.indexOf("--days");
const days = daysIndex >= 0 ? Number(args[daysIndex + 1]) : 30;
const since = Date.now() - days * 86_400_000;

const db = new Database(path, { readonly: true });
db.exec("PRAGMA query_only = 1");

// One row per completed turn after a human prompt. `renewed` marks a turn
// whose prompt followed a renewal marker in the same gap.
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
  )
  SELECT te.provider, te.input, te.cached, te.written, te.uncached,
    (SELECT min(x.ts_ms) FROM events x WHERE x.session_id = te.session_id
       AND x.seq > te.prev_seq AND x.seq < te.seq
       AND json_extract(x.payload, '$.update.sessionUpdate') = 'user_message_chunk'
       -- Human prompts only: renewal never applies to scheduler wakeups,
       -- managed-call notices or runtime prompts.
       AND coalesce(json_extract(x.payload, '$.update.promptOrigin.actor'), 'human') = 'human'
       AND coalesce(json_extract(x.payload, '$.update.autoResumed'), 0) = 0
       AND coalesce(json_extract(x.payload, '$.update.content.text'), '') NOT LIKE '<system-reminder>%')
      - te.prev_end AS gap_ms,
    EXISTS (SELECT 1 FROM events x WHERE x.session_id = te.session_id
       AND x.seq > te.prev_seq AND x.seq < te.seq
       AND json_extract(x.payload, '$.update.sessionUpdate') = 'context_rolled_over') AS renewed
  FROM te
  WHERE te.prev_end IS NOT NULL AND te.input > 0
`)
  .all(since) as {
  provider: string;
  input: number;
  cached: number | null;
  written: number | null;
  uncached: number | null;
  gap_ms: number | null;
  renewed: number;
}[];

const buckets: [string, number][] = [
  ["<5m", 5 * 60_000],
  ["5-30m", 30 * 60_000],
  ["30-60m", 60 * 60_000],
  ["1-3h", 3 * 3_600_000],
  ["3-12h", 12 * 3_600_000],
  [">12h", Number.POSITIVE_INFINITY],
];
const bucket = (gap: number): string => buckets.find(([, limit]) => gap < limit)![0];

interface Group {
  provider: string;
  gap: string;
  renewed: boolean;
  turns: number;
  input: number;
  written: number;
  fresh: number;
}
const groups = new Map<string, Group>();
for (const row of rows) {
  if (row.gap_ms === null) continue;
  const gap = bucket(row.gap_ms);
  const key = `${row.provider}|${gap}|${row.renewed}`;
  const group = groups.get(key) ?? {
    provider: row.provider,
    gap,
    renewed: row.renewed === 1,
    turns: 0,
    input: 0,
    written: 0,
    fresh: 0,
  };
  group.turns += 1;
  group.input += row.input;
  group.written += row.written ?? 0;
  group.fresh += (row.written ?? 0) + (row.uncached ?? 0);
  groups.set(key, group);
}
const report = [...groups.values()]
  .sort(
    (a, b) =>
      a.provider.localeCompare(b.provider) ||
      buckets.findIndex(([name]) => name === a.gap) -
        buckets.findIndex(([name]) => name === b.gap) ||
      Number(a.renewed) - Number(b.renewed),
  )
  .map((group) => ({
    provider: group.provider,
    gap: group.gap,
    renewed: group.renewed,
    turns: group.turns,
    mean_input: Math.round(group.input / group.turns),
    mean_cache_write: Math.round(group.written / group.turns),
    // Written plus uncached input: what the turn paid at more than read rate.
    fresh_ratio: Number((group.fresh / Math.max(1, group.input)).toFixed(3)),
  }));

if (args.includes("--json")) {
  console.log(JSON.stringify({ since_ms: since, days, groups: report }, null, 2));
} else {
  console.log(`Turns since ${new Date(since).toISOString()} (${days} days)`);
  console.table(report);
}
