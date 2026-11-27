// Field cost, quota and gateway telemetry, read from a Controller host.
//
// Usage (repository root, dev shell):
//   bun .agents/skills/model-intel/scripts/field.ts \
//     --database /var/lib/cowboy/cowboy.sqlite3 --days 14 --out models/field/2026-11-23.json
//
// Read-only: the database and usage snapshot files open read-only. Only
// aggregates per (Provider, purpose, resolved model, effort) or anonymized
// quota windows are emitted: no session, call, account UUID, owner, path or
// text. These are observational field data from a mixed workload, not a paired
// quality comparison; `score.ts` never scores them.

import { Database } from "bun:sqlite";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import type { RoutingTable } from "./score.ts";

export {
  type CacheFactInputRow,
  type CacheTelemetryAudit,
  type ProviderCacheAuditStat,
  readCacheTelemetryAudit,
  SHADOW_IDLE_GAP_BUCKETS,
  type ShadowIdleGapBucket,
  summarizeCacheTelemetryRows,
  type TelemetryCoverageRegression,
} from "./cache-telemetry.ts";

export type Call = {
  provider: string;
  purpose: string;
  model: string;
  effort: string;
  risk: string;
  selection?: string | null;
  tokens: number | null;
  uncached: number | null;
  minutes: number | null;
  verdict: string | null;
  findings: number | null;
};

export type Group = {
  provider: string;
  purpose: string;
  target: string;
  n: number;
  with_usage: number;
  median_tokens: number | null;
  p75_tokens: number | null;
  median_uncached_tokens: number | null;
  median_minutes: number | null;
  mean_findings: number | null;
  verdicts: Record<string, number>;
  risk: Record<string, number>;
};

export type CostDrift = {
  target: string;
  recorded_median_tokens: number;
  live_median_tokens: number;
  delta_ratio: number;
  n: number;
};

export type TelemetrySummary = {
  review_groups: Group[];
  analysis_groups: Group[];
  review_selection: Record<string, number>;
  review_risk: Record<string, number>;
  cost_drifts: CostDrift[];
};

export type DeepSeekGatewayStat = {
  agent: string;
  model_family: string;
  requests: number;
  median_total_tokens: number | null;
  cache_hit_ratio: number | null;
};

export type QuotaWindow = {
  provider: string;
  limit_id: string;
  used_percent: number;
  window_duration_mins: number | null;
  resets_at_ms: number | null;
  observed_at_ms: number;
  observed_age_mins: number;
  stale: boolean;
};

export type QuotaBurn = {
  provider: string;
  limit_id: string;
  delta_used_percent: number;
  elapsed_hours: number;
  burn_percent_per_day: number;
  calls_in_window: number;
  estimated_percent_per_call: number | null;
};

const quantile = (values: number[], q: number): number | null => {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const position = (sorted.length - 1) * q;
  const low = Math.floor(position);
  const high = Math.ceil(position);
  return Math.round(sorted[low]! + (sorted[high]! - sorted[low]!) * (position - low));
};

const count = (values: string[]): Record<string, number> => {
  const counts: Record<string, number> = {};
  for (const value of values) counts[value] = (counts[value] ?? 0) + 1;
  return counts;
};

export function aggregate(calls: Call[]): Group[] {
  const groups = new Map<string, Call[]>();
  for (const call of calls) {
    const key = [call.provider, call.purpose, `${call.model}:${call.effort}`].join("\u0000");
    groups.set(key, [...(groups.get(key) ?? []), call]);
  }
  return [...groups.entries()]
    .map(([key, members]) => {
      const [provider, purpose, target] = key.split("\u0000") as [string, string, string];
      const numbers = (pick: (call: Call) => number | null) =>
        members.map(pick).filter((value): value is number => value !== null);
      const findings = numbers((call) => call.findings);
      return {
        provider,
        purpose,
        target,
        n: members.length,
        with_usage: numbers((call) => call.tokens).length,
        // Token statistics count only calls that reported usage (`with_usage`).
        median_tokens: quantile(
          numbers((call) => call.tokens),
          0.5,
        ),
        p75_tokens: quantile(
          numbers((call) => call.tokens),
          0.75,
        ),
        median_uncached_tokens: quantile(
          numbers((call) => call.uncached),
          0.5,
        ),
        median_minutes: quantile(
          numbers((call) => call.minutes),
          0.5,
        ),
        mean_findings: findings.length
          ? Math.round((findings.reduce((a, b) => a + b, 0) / findings.length) * 10) / 10
          : null,
        verdicts: count(members.map((call) => call.verdict ?? "none")),
        risk: count(members.map((call) => call.risk)),
      };
    })
    .sort((a, b) => b.n - a.n);
}

export function detectCostDrifts(
  groups: Group[],
  routing: RoutingTable | undefined,
  options: { minSamples?: number; threshold?: number } = {},
): CostDrift[] {
  if (!routing) return [];
  const minSamples = options.minSamples ?? 5;
  const threshold = options.threshold ?? 0.3;
  const drifts: CostDrift[] = [];
  for (const group of groups) {
    if (group.with_usage < minSamples || group.median_tokens === null) continue;
    const field = routing.costs[group.target]?.field;
    if (!field || typeof field.median_tokens !== "number" || field.median_tokens <= 0) continue;
    if (field.purpose && field.purpose !== group.purpose) continue;
    const ratio = (group.median_tokens - field.median_tokens) / field.median_tokens;
    if (Math.abs(ratio) >= threshold) {
      drifts.push({
        target: group.target,
        recorded_median_tokens: field.median_tokens,
        live_median_tokens: group.median_tokens,
        delta_ratio: Math.round(ratio * 1000) / 1000,
        n: group.with_usage,
      });
    }
  }
  return drifts.sort((a, b) => Math.abs(b.delta_ratio) - Math.abs(a.delta_ratio));
}

export function summarizeTelemetry(
  calls: Call[],
  routing?: RoutingTable,
  options?: { minSamples?: number; threshold?: number },
): TelemetrySummary {
  const reviews = calls.filter((c) => c.purpose === "review" || c.purpose === "design_review");
  const analyses = calls.filter((c) => c.purpose === "analysis");
  const review_groups = aggregate(reviews);
  const analysis_groups = aggregate(analyses);
  return {
    review_groups,
    analysis_groups,
    review_selection: count(reviews.map((c) => c.selection ?? "unknown")),
    review_risk: count(reviews.map((c) => c.risk)),
    cost_drifts: detectCostDrifts([...review_groups, ...analysis_groups], routing, options),
  };
}

export function readLedgerCalls(
  database: string,
  since: number,
  purposes: string[] = ["review", "design_review"],
): Call[] {
  const db = new Database(database, { readonly: true });
  try {
    const placeholders = purposes.map(() => "?").join(",");
    const rows = db
      .query(
        `SELECT json_extract(document,'$.provider') AS provider,
                json_extract(document,'$.purpose') AS purpose,
                json_extract(document,'$.configuration.model') AS model,
                COALESCE(json_extract(document,'$.configuration.reasoning_effort'),
                         json_extract(document,'$.configuration.effort'),
                         json_extract(document,'$.configuration.thought_level'), 'default') AS effort,
                COALESCE(json_extract(request_document,'$.labels.review_risk'), 'undeclared') AS risk,
                json_extract(document,'$.selection') AS selection,
                json_extract(document,'$.result.usage.total_tokens') AS tokens,
                json_extract(document,'$.result.usage.uncached_input_tokens') AS uncached,
                (json_extract(document,'$.finished_at_ms') - json_extract(document,'$.started_at_ms')) / 60000.0 AS minutes,
                CASE
                  WHEN json_extract(document,'$.result.structured.verdict') IS NULL THEN NULL
                  WHEN json_extract(document,'$.result.structured.verdict')
                    IN ('approve','needs-attention','incomplete')
                    THEN json_extract(document,'$.result.structured.verdict')
                  ELSE 'other' END AS verdict,
                CASE WHEN json_type(document,'$.result.structured.findings') = 'array'
                  THEN json_array_length(document,'$.result.structured.findings') END AS findings
         FROM managed_agent_calls
         WHERE state = 'completed'
           AND json_extract(document,'$.purpose') IN (${placeholders})
           AND json_extract(document,'$.configuration.model') IS NOT NULL
           -- A reused receipt copies another run's outcome without running.
           AND json_extract(document,'$.result.reused_from') IS NULL
           AND json_extract(document,'$.created_at_ms') >= ?`,
      )
      .all(...purposes, since) as Call[];
    return rows.map((row) => ({
      ...row,
      minutes: row.minutes === null ? null : Math.round(row.minutes * 10) / 10,
    }));
  } finally {
    db.close();
  }
}

export function readCompletedCallCounts(
  database: string,
  sinceMs: number,
  untilMs: number,
): Record<string, number> {
  const db = new Database(database, { readonly: true });
  try {
    const rows = db
      .query(
        `SELECT json_extract(document,'$.provider') AS provider, COUNT(*) AS n
         FROM managed_agent_calls
         WHERE state = 'completed'
           AND json_extract(document,'$.result.reused_from') IS NULL
           AND json_extract(document,'$.created_at_ms') >= ?
           AND json_extract(document,'$.created_at_ms') <= ?
         GROUP BY 1`,
      )
      .all(sinceMs, untilMs) as Array<{ provider: string | null; n: number }>;
    const out: Record<string, number> = {};
    for (const row of rows) {
      if (row.provider) out[row.provider] = row.n;
    }
    return out;
  } finally {
    db.close();
  }
}

export function readDeepSeekGatewayStats(database: string, since: number): DeepSeekGatewayStat[] {
  const db = new Database(database, { readonly: true });
  try {
    const hasTable = db
      .query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'provider_usage_events'")
      .get();
    if (!hasTable) return [];
    const rows = db
      .query(
        `SELECT agent, model_family,
                COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0) AS total_tokens,
                COALESCE(cache_hit_tokens, 0) AS cache_hit,
                COALESCE(cache_miss_tokens, 0) AS cache_miss
         FROM provider_usage_events
         WHERE provider = 'deepseek'
           AND occurred_at_ms >= ?`,
      )
      .all(since) as Array<{
      agent: string;
      model_family: string;
      total_tokens: number;
      cache_hit: number;
      cache_miss: number;
    }>;
    const grouped = new Map<
      string,
      { agent: string; model_family: string; totals: number[]; hits: number; misses: number }
    >();
    for (const row of rows) {
      const key = `${row.agent}\u0000${row.model_family}`;
      const bucket = grouped.get(key) ?? {
        agent: row.agent,
        model_family: row.model_family,
        totals: [],
        hits: 0,
        misses: 0,
      };
      if (row.total_tokens > 0) bucket.totals.push(row.total_tokens);
      bucket.hits += row.cache_hit;
      bucket.misses += row.cache_miss;
      grouped.set(key, bucket);
    }
    return [...grouped.values()]
      .map((b) => {
        const cacheDenom = b.hits + b.misses;
        return {
          agent: b.agent,
          model_family: b.model_family,
          requests: b.totals.length || b.hits + b.misses > 0 ? Math.max(b.totals.length, 1) : 0,
          median_total_tokens: quantile(b.totals, 0.5),
          cache_hit_ratio: cacheDenom > 0 ? Math.round((b.hits / cacheDenom) * 1000) / 1000 : null,
        };
      })
      .sort((a, b) => b.requests - a.requests);
  } finally {
    db.close();
  }
}

type RawUsageNamed = {
  entries?: Array<{
    usage?: {
      provider?: string;
      status?: string;
      observed_at_ms?: number;
      rate_limits?: {
        rateLimitsByLimitId?: Record<
          string,
          {
            primary?: {
              usedPercent?: number;
              windowDurationMins?: number;
              resetsAt?: number;
            };
          }
        >;
      };
    };
  }>;
};

const USAGE_PROVIDER_TO_CALL_PROVIDER: Record<string, string> = {
  openai: "codex",
  anthropic: "claude-code",
  antigravity: "antigravity",
  xai: "grok",
  deepseek: "deepseek",
};

// Read anonymized quota windows from Cowboy's persisted named-account usage
// snapshot (`usage-named.json`). No owner IDs, account UUIDs, or credentials
// are retained; when multiple accounts share one (provider, limit_id), keep the
// freshest reading so pressure checks see current state.
export function readQuotaSnapshots(
  usagePath: string,
  nowMs: number = Date.now(),
  maxAgeMs = 30 * 60_000,
): QuotaWindow[] {
  if (!existsSync(usagePath)) return [];
  const raw = JSON.parse(readFileSync(usagePath, "utf8")) as RawUsageNamed;
  const best = new Map<string, QuotaWindow>();
  for (const entry of raw.entries ?? []) {
    const usage = entry.usage;
    if (!usage || usage.status !== "available" || !usage.provider) continue;
    const observed = typeof usage.observed_at_ms === "number" ? usage.observed_at_ms : 0;
    if (observed <= 0) continue;
    const limits = usage.rate_limits?.rateLimitsByLimitId;
    if (!limits) continue;
    const ageMs = Math.max(0, nowMs - observed);
    for (const [limitId, detail] of Object.entries(limits)) {
      const primary = detail?.primary;
      if (!primary || typeof primary.usedPercent !== "number") continue;
      const resetsAtMs =
        typeof primary.resetsAt === "number" && primary.resetsAt > 0
          ? Math.round(primary.resetsAt > 1e12 ? primary.resetsAt : primary.resetsAt * 1000)
          : null;
      const window: QuotaWindow = {
        provider: usage.provider,
        limit_id: limitId,
        used_percent: Math.round(primary.usedPercent * 10) / 10,
        window_duration_mins:
          typeof primary.windowDurationMins === "number" ? primary.windowDurationMins : null,
        resets_at_ms: resetsAtMs,
        observed_at_ms: observed,
        observed_age_mins: Math.round(ageMs / 60_000),
        stale: ageMs > maxAgeMs,
      };
      const key = `${window.provider}\u0000${window.limit_id}`;
      const prev = best.get(key);
      if (!prev || window.observed_at_ms > prev.observed_at_ms) {
        best.set(key, window);
      }
    }
  }
  return [...best.values()].sort(
    (a, b) => a.provider.localeCompare(b.provider) || a.limit_id.localeCompare(b.limit_id),
  );
}

// Compute quota burn rates across consecutive observations in the same reset
// window (`resets_at_ms` equal within 60s and `used_percent` non-decreasing).
export function computeQuotaBurn(
  previous: QuotaWindow[],
  current: QuotaWindow[],
  callsForRange?: (provider: string, sinceMs: number, untilMs: number) => number,
): QuotaBurn[] {
  const prevByKey = new Map<string, QuotaWindow>();
  for (const row of previous) {
    const key = `${row.provider}\u0000${row.limit_id}`;
    const existing = prevByKey.get(key);
    if (!existing || row.observed_at_ms > existing.observed_at_ms) {
      prevByKey.set(key, row);
    }
  }
  const burns: QuotaBurn[] = [];
  for (const cur of current) {
    if (cur.stale) continue;
    const prev = prevByKey.get(`${cur.provider}\u0000${cur.limit_id}`);
    if (!prev) continue;
    const elapsedMs = cur.observed_at_ms - prev.observed_at_ms;
    if (elapsedMs < 60_000) continue;
    const sameReset =
      cur.resets_at_ms !== null &&
      prev.resets_at_ms !== null &&
      Math.abs(cur.resets_at_ms - prev.resets_at_ms) <= 60_000;
    if (!sameReset) continue;
    const delta = Math.round((cur.used_percent - prev.used_percent) * 10) / 10;
    if (delta < 0) continue;
    const elapsedHours = Math.round((elapsedMs / 3_600_000) * 100) / 100;
    const burnPerDay = Math.round((delta / (elapsedMs / 86_400_000)) * 10) / 10;
    const callProvider = USAGE_PROVIDER_TO_CALL_PROVIDER[cur.provider] ?? cur.provider;
    const calls = callsForRange
      ? callsForRange(callProvider, prev.observed_at_ms, cur.observed_at_ms)
      : 0;
    burns.push({
      provider: cur.provider,
      limit_id: cur.limit_id,
      delta_used_percent: delta,
      elapsed_hours: elapsedHours,
      burn_percent_per_day: burnPerDay,
      calls_in_window: calls,
      estimated_percent_per_call: calls > 0 ? Math.round((delta / calls) * 1000) / 1000 : null,
    });
  }
  return burns;
}

export function writeFileGuarded(targetPath: string, text: string, protectedDatabase?: string) {
  const target = resolve(targetPath);
  mkdirSync(dirname(target), { recursive: true });
  if (protectedDatabase) {
    // Never overwrite the ledger (or its WAL/SHM files) through a typo or link.
    const ledger = realpathSync(protectedDatabase);
    const real = (() => {
      try {
        return realpathSync(target);
      } catch {
        return join(realpathSync(dirname(target)), basename(target));
      }
    })();
    if ([ledger, `${ledger}-wal`, `${ledger}-shm`, `${ledger}-journal`].includes(real)) {
      throw new Error("--out must not name the ledger database");
    }
  }
  const staged = `${target}.tmp-${process.pid}-${Date.now()}`;
  // Exclusive creation: a pre-existing (possibly linked) staging path fails.
  writeFileSync(staged, text, { flag: "wx" });
  renameSync(staged, target);
}

const DATED_SNAPSHOT_RE = /^\d{4}-\d{2}-\d{2}\.json$/;

// Keep only the latest dated snapshot in `models/field/`; Git history retains older ones.
export function pruneOlderSnapshots(keepPath: string): string[] {
  const target = resolve(keepPath);
  const keepName = basename(target);
  if (!DATED_SNAPSHOT_RE.test(keepName)) return [];
  const dir = dirname(target);
  if (!existsSync(dir)) return [];
  const removed: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    if (name === keepName || !DATED_SNAPSHOT_RE.test(name)) continue;
    unlinkSync(join(dir, name));
    removed.push(name);
  }
  return removed;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const option = (name: string) => {
    const index = args.indexOf(name);
    return index >= 0 ? args[index + 1] : undefined;
  };
  const database = option("--database") ?? "/var/lib/cowboy/cowboy.sqlite3";
  const days = Number(option("--days") ?? "14");
  const out = option("--out");
  if (!Number.isFinite(days) || days <= 0) throw new Error("--days must be a positive number");
  const now = Date.now();
  const since = now - days * 86_400_000;
  const report = {
    schema: 1,
    measured_at: new Date(now).toISOString().slice(0, 10),
    window_days: days,
    source:
      "Hawk Controller managed_agent_calls (read-only), completed reviews, design reviews and analyses",
    caveat:
      "Observational mixed workload: inputs differ between groups, so this compares cost, not quality.",
    groups: aggregate(readLedgerCalls(database, since, ["review", "design_review"])),
    analysis_groups: aggregate(readLedgerCalls(database, since, ["analysis"])),
  };
  const text = `${JSON.stringify(report, null, 2)}\n`;
  if (out) {
    writeFileGuarded(out, text, database);
    pruneOlderSnapshots(out);
  } else {
    process.stdout.write(text);
  }
}
