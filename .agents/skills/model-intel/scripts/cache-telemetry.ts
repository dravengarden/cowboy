// Read-only turn-level prompt-cache telemetry audit, per-version & time-window
// coverage regression detection (usage, cache read, and cache write), and
// stratified shadow idle-gap diagnostic curves.
//
// Strictly anonymized: session IDs are counted only in memory to compute
// `distinct_sessions` and never emitted in reports.

import { Database } from "bun:sqlite";
import { normalizeTurnCacheUsage } from "../../../../tools/context-rollover-report.ts";

export const SHADOW_IDLE_GAP_BUCKETS: ReadonlyArray<readonly [string, number]> = [
  ["<5m", 5 * 60_000],
  ["5-10m", 10 * 60_000],
  ["10-20m", 20 * 60_000],
  ["20-30m", 30 * 60_000],
  ["30-60m", 60 * 60_000],
  ["1-3h", 3 * 3_600_000],
  ["3-12h", 12 * 3_600_000],
  [">12h", Number.POSITIVE_INFINITY],
];

export type CacheFactInputRow = {
  session_key: string;
  occurred_at_ms: number;
  provider: string;
  provider_version: string;
  model: string | null;
  stop_reason: string;
  prompt_origin: string;
  idle_gap_ms: number | null;
  renewed: boolean;
  usage_observed: boolean;
  inclusive_input_tokens: number | null;
  cached_read_tokens: number | null;
  cache_write_tokens: number | null;
  uncached_input_tokens: number | null;
};

export type ProviderCacheAuditStat = {
  provider: string;
  turns: number;
  eligible_turns: number;
  observed_usage_turns: number;
  observed_cache_turns: number;
  observed_cache_write_turns: number;
  renewed_turns: number;
  inclusive_input_tokens: number | null;
  cached_read_tokens: number | null;
  cache_write_tokens: number | null;
  uncached_input_tokens: number | null;
  usage_coverage: number | null;
  cache_coverage: number | null;
  cache_write_coverage: number | null;
  cached_input_share: number | null;
};

export type TelemetryCoverageRegression = {
  provider: string;
  comparison_kind: "version_cohort" | "time_window";
  metric: "usage_coverage" | "cache_coverage" | "cache_write_coverage";
  previous_version: string;
  current_version: string;
  previous_coverage: number;
  current_coverage: number;
  drop_ratio: number;
  previous_turns: number;
  current_turns: number;
};

export type ShadowIdleGapBucket = {
  provider: string;
  model: string;
  renewed: boolean;
  gap_bucket: string;
  turns: number;
  observed_cache_turns: number;
  distinct_sessions: number;
  distinct_days: number;
  mean_inclusive_input: number;
  mean_uncached_input: number | null;
  cached_input_share: number | null;
};

export type CacheTelemetryAudit = {
  source: "turn_cache_facts" | "events_fallback";
  projection_complete: boolean;
  granularity: "turn_total";
  shadow_diagnostic_note: string;
  providers: ProviderCacheAuditStat[];
  coverage_regressions: TelemetryCoverageRegression[];
  shadow_ttl_curves: ShadowIdleGapBucket[];
};

const SHADOW_NOTE =
  "Shadow turn-level diagnostic only (multi-step turns summed); advisory evidence for upgrade audits, never an automatic default mutator.";

function gapBucketLabel(gapMs: number): string {
  for (const [label, upper] of SHADOW_IDLE_GAP_BUCKETS) {
    if (gapMs < upper) return label;
  }
  return ">12h";
}

function isEligibleStopReason(stopReason: string, usageObserved: boolean): boolean {
  if (usageObserved) return true;
  const lowered = stopReason.trim().toLowerCase();
  return (
    lowered !== "cancelled" &&
    lowered !== "canceled" &&
    lowered !== "error" &&
    !lowered.startsWith("error:")
  );
}

function roundRatio(n: number, d: number): number | null {
  if (d <= 0) return null;
  return Math.round((n / d) * 10_000) / 10_000;
}

type CohortCounts = {
  eligible: number;
  usage: number;
  cache: number;
  write: number;
  firstSeenMs: number;
  lastSeenMs: number;
};

function compareCohorts(
  provider: string,
  comparisonKind: "version_cohort" | "time_window",
  prevLabel: string,
  currLabel: string,
  prev: CohortCounts,
  curr: CohortCounts,
  dropThreshold: number,
  out: TelemetryCoverageRegression[],
): void {
  const checks: Array<{
    metric: "usage_coverage" | "cache_coverage" | "cache_write_coverage";
    prevCov: number;
    currCov: number;
    minBaseline: number;
  }> = [
    {
      metric: "usage_coverage",
      prevCov: prev.usage / prev.eligible,
      currCov: curr.usage / curr.eligible,
      minBaseline: 0,
    },
    {
      metric: "cache_coverage",
      prevCov: prev.cache / prev.eligible,
      currCov: curr.cache / curr.eligible,
      minBaseline: 0,
    },
    {
      metric: "cache_write_coverage",
      prevCov: prev.write / prev.eligible,
      currCov: curr.write / curr.eligible,
      // Alert on write-counter loss only when baseline actually reported writes.
      minBaseline: 0.5,
    },
  ];
  for (const check of checks) {
    if (check.prevCov < check.minBaseline) continue;
    const drop = check.prevCov - check.currCov;
    if (drop > dropThreshold) {
      out.push({
        provider,
        comparison_kind: comparisonKind,
        metric: check.metric,
        previous_version: prevLabel,
        current_version: currLabel,
        previous_coverage: Math.round(check.prevCov * 10_000) / 10_000,
        current_coverage: Math.round(check.currCov * 10_000) / 10_000,
        drop_ratio: Math.round(drop * 10_000) / 10_000,
        previous_turns: prev.eligible,
        current_turns: curr.eligible,
      });
    }
  }
}

type SemverTuple = readonly [number, number, number, string | null];

function parseSemver(version: string): SemverTuple | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(
    version.trim(),
  );
  if (!match) return null;
  return [
    Number.parseInt(match[1]!, 10),
    Number.parseInt(match[2]!, 10),
    Number.parseInt(match[3]!, 10),
    match[4] ?? null,
  ];
}

function comparePrerelease(a: string, b: string): number {
  const aParts = a.split(".");
  const bParts = b.split(".");
  const len = Math.min(aParts.length, bParts.length);
  const numeric = /^\d+$/;
  for (let i = 0; i < len; i++) {
    const ai = aParts[i]!;
    const bi = bParts[i]!;
    if (ai === bi) continue;
    const aNum = numeric.test(ai);
    const bNum = numeric.test(bi);
    if (aNum && bNum) {
      const aBig = BigInt(ai);
      const bBig = BigInt(bi);
      return aBig < bBig ? -1 : 1;
    }
    if (aNum !== bNum) return aNum ? -1 : 1;
    return ai < bi ? -1 : 1;
  }
  return aParts.length - bParts.length;
}

function compareSemver(a: SemverTuple, b: SemverTuple): number {
  if (a[0] !== b[0]) return a[0] - b[0];
  if (a[1] !== b[1]) return a[1] - b[1];
  if (a[2] !== b[2]) return a[2] - b[2];
  if (a[3] === b[3]) return 0;
  if (a[3] === null) return 1;
  if (b[3] === null) return -1;
  return comparePrerelease(a[3], b[3]);
}

/** First-party providers whose ACP/gateway surfaces default unsupported cache_write_tokens to 0. */
export const UNMETERED_ZERO_CACHE_WRITE_PROVIDERS = new Set([
  "codex",
  "antigravity",
  "claude-deepseek",
  "codex-deepseek",
  "deepseek-harness",
  "grok",
]);

function normalizeCacheWriteForProvider(provider: string, written: number | null): number | null {
  if (written === 0 && UNMETERED_ZERO_CACHE_WRITE_PROVIDERS.has(provider)) {
    return null;
  }
  return written;
}

export function summarizeCacheTelemetryRows(
  rows: readonly CacheFactInputRow[],
  nowMs: number,
  windowDays: number,
  source: "turn_cache_facts" | "events_fallback",
  options: {
    minRegressionTurns?: number;
    dropThreshold?: number;
    projectionComplete?: boolean;
    /** Full-history `MIN(occurred_at_ms)` keyed by `${provider}\u0000${version}`. */
    versionFirstSeenMs?: ReadonlyMap<string, number>;
  } = {},
): CacheTelemetryAudit {
  const minTurns = options.minRegressionTurns ?? 10;
  const dropThreshold = options.dropThreshold ?? 0.15;
  const projectionComplete = options.projectionComplete ?? source === "turn_cache_facts";
  const halfBoundaryMs = nowMs - (windowDays * 86_400_000) / 2;

  type ProvAcc = {
    turns: number;
    eligible: number;
    usageTurns: number;
    cacheTurns: number;
    writeTurns: number;
    renewedTurns: number;
    inclusiveInput: number;
    observedCacheInput: number;
    cachedRead: number;
    cacheWrite: number;
    uncachedTotal: number;
    prevHalf: CohortCounts;
    currHalf: CohortCounts;
    versionCohorts: Map<string, CohortCounts>;
  };

  type GapAcc = {
    provider: string;
    model: string;
    renewed: boolean;
    gap_bucket: string;
    turns: number;
    observedCacheTurns: number;
    sessions: Set<string>;
    days: Set<string>;
    inputSum: number;
    observedCacheInput: number;
    cachedReadSum: number;
    uncachedSum: number;
  };

  const emptyCohort = (): CohortCounts => ({
    eligible: 0,
    usage: 0,
    cache: 0,
    write: 0,
    firstSeenMs: Number.POSITIVE_INFINITY,
    lastSeenMs: Number.NEGATIVE_INFINITY,
  });

  const recordCohort = (
    cohort: CohortCounts,
    tsMs: number,
    usageValid: boolean,
    cacheValid: boolean,
    writeValid: boolean,
  ): void => {
    cohort.eligible += 1;
    if (usageValid) cohort.usage += 1;
    if (cacheValid) cohort.cache += 1;
    if (writeValid) cohort.write += 1;
    if (tsMs < cohort.firstSeenMs) cohort.firstSeenMs = tsMs;
    if (tsMs > cohort.lastSeenMs) cohort.lastSeenMs = tsMs;
  };

  const providers = new Map<string, ProvAcc>();
  const gaps = new Map<string, GapAcc>();

  for (const row of rows) {
    const normalized =
      row.usage_observed && row.inclusive_input_tokens !== null
        ? normalizeTurnCacheUsage({
            input: row.inclusive_input_tokens,
            cached: row.cached_read_tokens,
            written: normalizeCacheWriteForProvider(row.provider, row.cache_write_tokens),
            uncached: row.uncached_input_tokens,
          })
        : null;
    const usageValid = normalized !== null;
    const cacheValid =
      normalized !== null && normalized.cached_read !== null && normalized.uncached_total !== null;
    const writeValid = normalized !== null && normalized.cache_write !== null;
    const eligible = isEligibleStopReason(row.stop_reason, usageValid);

    const acc = providers.get(row.provider) ?? {
      turns: 0,
      eligible: 0,
      usageTurns: 0,
      cacheTurns: 0,
      writeTurns: 0,
      renewedTurns: 0,
      inclusiveInput: 0,
      observedCacheInput: 0,
      cachedRead: 0,
      cacheWrite: 0,
      uncachedTotal: 0,
      prevHalf: emptyCohort(),
      currHalf: emptyCohort(),
      versionCohorts: new Map(),
    };
    acc.turns += 1;
    if (eligible) acc.eligible += 1;
    if (row.renewed) acc.renewedTurns += 1;
    if (usageValid) {
      acc.usageTurns += 1;
      acc.inclusiveInput += normalized.inclusive_input;
    }
    if (cacheValid) {
      acc.cacheTurns += 1;
      acc.observedCacheInput += normalized.inclusive_input;
      acc.cachedRead += normalized.cached_read!;
      acc.uncachedTotal += normalized.uncached_total!;
    }
    if (writeValid) {
      acc.writeTurns += 1;
      acc.cacheWrite += normalized!.cache_write!;
    }
    if (eligible) {
      const half = row.occurred_at_ms >= halfBoundaryMs ? acc.currHalf : acc.prevHalf;
      recordCohort(half, row.occurred_at_ms, usageValid, cacheValid, writeValid);
      const version = row.provider_version.trim();
      if (version.length > 0) {
        const cohort = acc.versionCohorts.get(version) ?? emptyCohort();
        recordCohort(cohort, row.occurred_at_ms, usageValid, cacheValid, writeValid);
        acc.versionCohorts.set(version, cohort);
      }
    }
    providers.set(row.provider, acc);

    // Shadow idle-gap curve: human-origin turns with known non-negative gap & input > 0.
    if (
      usageValid &&
      normalized.inclusive_input > 0 &&
      row.prompt_origin === "human" &&
      row.idle_gap_ms !== null &&
      row.idle_gap_ms >= 0
    ) {
      const model = row.model?.trim() || "unknown";
      const bucket = gapBucketLabel(row.idle_gap_ms);
      const key = `${row.provider}\u0000${model}\u0000${row.renewed ? 1 : 0}\u0000${bucket}`;
      const g = gaps.get(key) ?? {
        provider: row.provider,
        model,
        renewed: row.renewed,
        gap_bucket: bucket,
        turns: 0,
        observedCacheTurns: 0,
        sessions: new Set<string>(),
        days: new Set<string>(),
        inputSum: 0,
        observedCacheInput: 0,
        cachedReadSum: 0,
        uncachedSum: 0,
      };
      g.turns += 1;
      g.sessions.add(row.session_key);
      g.days.add(new Date(row.occurred_at_ms).toISOString().slice(0, 10));
      g.inputSum += normalized.inclusive_input;
      if (cacheValid) {
        g.observedCacheTurns += 1;
        g.observedCacheInput += normalized.inclusive_input;
        g.cachedReadSum += normalized.cached_read!;
        g.uncachedSum += normalized.uncached_total!;
      }
      gaps.set(key, g);
    }
  }

  const providerStats: ProviderCacheAuditStat[] = [...providers.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([provider, acc]) => ({
      provider,
      turns: acc.turns,
      eligible_turns: acc.eligible,
      observed_usage_turns: acc.usageTurns,
      observed_cache_turns: acc.cacheTurns,
      observed_cache_write_turns: acc.writeTurns,
      renewed_turns: acc.renewedTurns,
      inclusive_input_tokens: acc.usageTurns > 0 ? acc.inclusiveInput : null,
      cached_read_tokens: acc.cacheTurns > 0 ? acc.cachedRead : null,
      cache_write_tokens: acc.writeTurns > 0 ? acc.cacheWrite : null,
      uncached_input_tokens: acc.cacheTurns > 0 ? acc.uncachedTotal : null,
      usage_coverage: roundRatio(acc.usageTurns, acc.eligible),
      cache_coverage: roundRatio(acc.cacheTurns, acc.eligible),
      cache_write_coverage: roundRatio(acc.writeTurns, acc.eligible),
      cached_input_share: roundRatio(acc.cachedRead, acc.observedCacheInput),
    }));

  const coverageRegressions: TelemetryCoverageRegression[] = [];
  for (const [provider, acc] of [...providers.entries()].sort(([a], [b]) => a.localeCompare(b))) {
    // 1. Compare exact per-version cohorts using rollout-safe ordering:
    //    (a) strict semver when every qualifying version parses as semver
    //        (immune to overlapping rolling upgrades);
    //    (b) full-table first-seen timestamps outside the report window;
    //    (c) strictly non-overlapping in-window intervals.
    //    If non-semver cohorts overlap with unknown order, skip directional
    //    version_cohort comparison and let the unoriented time_window check run.
    const eligibleVersions = [...acc.versionCohorts.entries()].filter(
      ([, cohort]) => cohort.eligible >= minTurns,
    );
    let orderedVersions: Array<[string, CohortCounts]> = [];
    if (eligibleVersions.length >= 2) {
      const parsed = eligibleVersions.map(([v, c]) => [v, c, parseSemver(v)] as const);
      const semverSorted = parsed.every(([, , sv]) => sv !== null)
        ? [...parsed].sort((a, b) => compareSemver(a[2]!, b[2]!))
        : null;
      const semverStrictlyOrdered =
        semverSorted !== null &&
        semverSorted.every(
          (entry, idx) => idx === 0 || compareSemver(semverSorted[idx - 1]![2]!, entry[2]!) < 0,
        );
      if (semverStrictlyOrdered && semverSorted !== null) {
        orderedVersions = semverSorted.map(([v, c]) => [v, c]);
      } else if (
        options.versionFirstSeenMs &&
        eligibleVersions.every(([v]) => options.versionFirstSeenMs!.has(`${provider}\u0000${v}`))
      ) {
        const withFullFirst = eligibleVersions.map(
          ([v, c]) => [v, c, options.versionFirstSeenMs!.get(`${provider}\u0000${v}`)!] as const,
        );
        const sorted = [...withFullFirst].sort((a, b) => a[2] - b[2] || a[0].localeCompare(b[0]));
        const strictlyOrdered = sorted.every(
          (entry, idx) => idx === 0 || sorted[idx - 1]![2] < entry[2],
        );
        if (strictlyOrdered) {
          orderedVersions = sorted.map(([v, c]) => [v, c]);
        }
      } else {
        const byStart = [...eligibleVersions].sort(
          (a, b) => a[1].firstSeenMs - b[1].firstSeenMs || a[0].localeCompare(b[0]),
        );
        const nonOverlapping = byStart.every(
          (entry, idx) => idx === 0 || byStart[idx - 1]![1].lastSeenMs <= entry[1].firstSeenMs,
        );
        if (nonOverlapping) {
          orderedVersions = byStart;
        }
      }
    }
    let detectedForProvider = false;
    if (orderedVersions.length >= 2) {
      for (let i = 1; i < orderedVersions.length; i++) {
        const [currVersion, currCohort] = orderedVersions[i]!;
        const [prevVersion, prevCohort] = orderedVersions[i - 1]!;
        const beforeCount = coverageRegressions.length;
        compareCohorts(
          provider,
          "version_cohort",
          prevVersion,
          currVersion,
          prevCohort,
          currCohort,
          dropThreshold,
          coverageRegressions,
        );
        if (coverageRegressions.length > beforeCount) detectedForProvider = true;
      }
    }
    // 2. Separate time-window trend check (explicitly labelled `time_window`,
    // never misattributing a mixed window to a single version string).
    if (
      !detectedForProvider &&
      acc.prevHalf.eligible >= minTurns &&
      acc.currHalf.eligible >= minTurns
    ) {
      compareCohorts(
        provider,
        "time_window",
        "prev_half",
        "recent_half",
        acc.prevHalf,
        acc.currHalf,
        dropThreshold,
        coverageRegressions,
      );
    }
  }

  const bucketRank = new Map(SHADOW_IDLE_GAP_BUCKETS.map(([name], index) => [name, index]));
  const shadowCurves: ShadowIdleGapBucket[] = [...gaps.values()]
    .sort(
      (a, b) =>
        a.provider.localeCompare(b.provider) ||
        a.model.localeCompare(b.model) ||
        Number(a.renewed) - Number(b.renewed) ||
        (bucketRank.get(a.gap_bucket) ?? 99) - (bucketRank.get(b.gap_bucket) ?? 99),
    )
    .map((g) => ({
      provider: g.provider,
      model: g.model,
      renewed: g.renewed,
      gap_bucket: g.gap_bucket,
      turns: g.turns,
      observed_cache_turns: g.observedCacheTurns,
      distinct_sessions: g.sessions.size,
      distinct_days: g.days.size,
      mean_inclusive_input: Math.round(g.inputSum / g.turns),
      mean_uncached_input:
        g.observedCacheTurns > 0 ? Math.round(g.uncachedSum / g.observedCacheTurns) : null,
      cached_input_share: roundRatio(g.cachedReadSum, g.observedCacheInput),
    }));

  return {
    source,
    projection_complete: projectionComplete,
    granularity: "turn_total",
    shadow_diagnostic_note: SHADOW_NOTE,
    providers: providerStats,
    coverage_regressions: coverageRegressions,
    shadow_ttl_curves: shadowCurves,
  };
}

function classifyOpenerOrigin(payloadJson: string | null): string {
  if (!payloadJson) return "unknown";
  try {
    const parsed = JSON.parse(payloadJson) as {
      update?: {
        promptOrigin?: { actor?: unknown };
        autoResumed?: unknown;
        content?: { text?: unknown };
      };
    };
    const update = parsed.update;
    if (!update) return "unknown";
    if (typeof update.promptOrigin?.actor === "string" && update.promptOrigin.actor.trim()) {
      return update.promptOrigin.actor.trim().toLowerCase();
    }
    if (update.autoResumed === true) return "auto_resume";
    if (
      typeof update.content?.text === "string" &&
      update.content.text.startsWith("<system-reminder>")
    ) {
      return "system_reminder";
    }
    return "human";
  } catch {
    return "unknown";
  }
}

export function readCacheTelemetryAudit(
  database: string,
  days: number,
  nowMs: number,
): CacheTelemetryAudit {
  const since = nowMs - days * 86_400_000;
  const db = new Database(database, { readonly: true });
  let inTx = false;
  try {
    db.exec("PRAGMA query_only = 1");
    db.exec("BEGIN DEFERRED");
    inTx = true;
    const hasFacts = db
      .query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'turn_cache_facts'")
      .get();
    const hasEvents = db
      .query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'events'")
      .get();
    const hasSessions = db
      .query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions'")
      .get();
    let projectionComplete = false;
    if (hasFacts) {
      const hasState = db
        .query(
          "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'turn_cache_backfill_state'",
        )
        .get();
      if (hasState) {
        const stateRow = db
          .query("SELECT completed_at_ms FROM turn_cache_backfill_state WHERE slot = 1")
          .get() as { completed_at_ms: number | null } | null;
        projectionComplete = stateRow !== null && stateRow.completed_at_ms !== null;
      }
      if (projectionComplete && hasEvents && hasSessions) {
        const hasIncarnationCol = (
          db.query("PRAGMA table_info(turn_cache_facts)").all() as Array<{ name: string }>
        ).some((col) => col.name === "session_created_at_ms");
        const hasSessionCreatedCol = (
          db.query("PRAGMA table_info(sessions)").all() as Array<{ name: string }>
        ).some((col) => col.name === "created_at_ms");
        const joinIncarnation =
          hasIncarnationCol && hasSessionCreatedCol
            ? " AND f.session_created_at_ms = s.created_at_ms"
            : "";
        const unprojected = db
          .query(
            `SELECT 1
             FROM events e
             JOIN sessions s ON s.id = e.session_id
             LEFT JOIN turn_cache_facts f
               ON f.session_id = e.session_id${joinIncarnation}
              AND f.turn_end_seq = e.seq
             WHERE json_extract(e.payload, '$.kind') = 'turn_end'
               AND e.ts_ms >= ?
               AND f.session_id IS NULL
             LIMIT 1`,
          )
          .get(since);
        if (unprojected !== null) {
          projectionComplete = false;
        }
      }
    }

    if (hasFacts && projectionComplete) {
      const firstSeenRows = db
        .query(
          `SELECT provider, provider_version, MIN(occurred_at_ms) AS first_seen_ms
           FROM turn_cache_facts
           WHERE provider_version != ''
           GROUP BY provider, provider_version`,
        )
        .all() as Array<{ provider: string; provider_version: string; first_seen_ms: number }>;
      const versionFirstSeenMs = new Map<string, number>();
      for (const row of firstSeenRows) {
        versionFirstSeenMs.set(`${row.provider}\u0000${row.provider_version}`, row.first_seen_ms);
      }
      const raw = db
        .query(
          `SELECT session_id, occurred_at_ms, provider, provider_version, model,
                  stop_reason, prompt_origin, idle_gap_ms, renewed, usage_observed,
                  inclusive_input_tokens, cached_read_tokens, cache_write_tokens,
                  CASE
                    WHEN inclusive_input_tokens IS NOT NULL AND cached_read_tokens IS NOT NULL
                    THEN inclusive_input_tokens - cached_read_tokens
                    ELSE NULL
                  END AS uncached_input_tokens
           FROM turn_cache_facts
           WHERE occurred_at_ms >= ?`,
        )
        .all(since) as Array<{
        session_id: string;
        occurred_at_ms: number;
        provider: string;
        provider_version: string;
        model: string | null;
        stop_reason: string;
        prompt_origin: string;
        idle_gap_ms: number | null;
        renewed: number;
        usage_observed: number;
        inclusive_input_tokens: number | null;
        cached_read_tokens: number | null;
        cache_write_tokens: number | null;
        uncached_input_tokens: number | null;
      }>;
      const rows: CacheFactInputRow[] = raw.map((r) => ({
        session_key: r.session_id,
        occurred_at_ms: r.occurred_at_ms,
        provider: r.provider,
        provider_version: r.provider_version ?? "",
        model: r.model,
        stop_reason: r.stop_reason,
        prompt_origin: r.prompt_origin,
        idle_gap_ms: r.idle_gap_ms,
        renewed: r.renewed === 1,
        usage_observed: r.usage_observed === 1,
        inclusive_input_tokens: r.inclusive_input_tokens,
        cached_read_tokens: r.cached_read_tokens,
        cache_write_tokens: r.cache_write_tokens,
        uncached_input_tokens: r.uncached_input_tokens,
      }));
      return summarizeCacheTelemetryRows(rows, nowMs, days, "turn_cache_facts", {
        projectionComplete: true,
        versionFirstSeenMs,
      });
    }

    if (!hasEvents || !hasSessions) {
      return summarizeCacheTelemetryRows([], nowMs, days, "events_fallback", {
        projectionComplete: false,
      });
    }

    // Fallback reader matches queries.rs exactly:
    // - Find the nearest preceding canonical TurnEnd by `(session_id, seq)`
    //   across ALL history (not filtered by `since`) so the first in-window turn
    //   never scans back into an earlier turn's opener or renewal marker.
    // - Honor `context_cleared` boundaries between `prev_seq` and `seq`.
    // - Locate the FIRST non-steering `user_message_chunk` after the boundary.
    // - Leave historical `provider_version` and `model` unknown (`""` / `null`)
    //   so pre-switch turns are never misattributed to current session metadata.
    const raw = db
      .query(
        `WITH te AS (
          SELECT e.session_id, e.seq, e.ts_ms, s.provider,
                 COALESCE(json_extract(e.payload, '$.stop_reason'), 'unknown') AS stop_reason,
                 json_extract(e.payload, '$.usage.input_tokens') AS inclusive_input_tokens,
                 json_extract(e.payload, '$.usage.cached_input_tokens') AS cached_read_tokens,
                 json_extract(e.payload, '$.usage.cache_write_tokens') AS cache_write_tokens,
                 json_extract(e.payload, '$.usage.uncached_input_tokens') AS uncached_input_tokens,
                 (SELECT prev.seq FROM events prev
                  WHERE prev.session_id = e.session_id
                    AND prev.seq < e.seq
                    AND json_extract(prev.payload, '$.kind') = 'turn_end'
                  ORDER BY prev.seq DESC LIMIT 1) AS prev_seq
          FROM events e
          JOIN sessions s ON s.id = e.session_id
          WHERE json_extract(e.payload, '$.kind') = 'turn_end' AND e.ts_ms >= ?1
        ),
        with_bounds AS (
          SELECT te.*,
                 prev.ts_ms AS raw_prev_end,
                 (SELECT MAX(c.seq) FROM events c
                  WHERE c.session_id = te.session_id
                    AND c.seq > COALESCE(te.prev_seq, -1) AND c.seq < te.seq
                    AND json_extract(c.payload, '$.kind') = 'update'
                    AND json_extract(c.payload, '$.update.sessionUpdate') = 'context_cleared') AS cleared_seq
          FROM te
          LEFT JOIN events prev ON prev.session_id = te.session_id AND prev.seq = te.prev_seq
        ),
        with_opener AS (
          SELECT b.*,
                 COALESCE(b.cleared_seq, b.prev_seq, -1) AS window_start,
                 CASE WHEN b.cleared_seq IS NOT NULL THEN NULL ELSE b.raw_prev_end END AS prev_end,
                 (SELECT x.seq FROM events x
                  WHERE x.session_id = b.session_id
                    AND x.seq > COALESCE(b.cleared_seq, b.prev_seq, -1) AND x.seq < b.seq
                    AND json_extract(x.payload, '$.kind') = 'update'
                    AND json_extract(x.payload, '$.update.sessionUpdate') = 'user_message_chunk'
                    AND COALESCE(json_extract(x.payload, '$.update._meta.cowboyDelivery'), '') != 'current_turn'
                  ORDER BY x.seq ASC LIMIT 1) AS opener_seq
          FROM with_bounds b
        )
        SELECT o.session_id, o.ts_ms AS occurred_at_ms, o.provider,
               o.stop_reason,
               o.inclusive_input_tokens, o.cached_read_tokens,
               o.cache_write_tokens, o.uncached_input_tokens,
               p.payload AS opener_payload,
               CASE
                 WHEN o.prev_end IS NOT NULL AND p.ts_ms IS NOT NULL AND p.ts_ms >= o.prev_end
                 THEN p.ts_ms - o.prev_end
                 ELSE NULL
               END AS idle_gap_ms,
               EXISTS (
                 SELECT 1 FROM events x
                 WHERE x.session_id = o.session_id
                   AND x.seq > o.window_start AND x.seq < o.seq
                   AND json_extract(x.payload, '$.kind') = 'update'
                   AND json_extract(x.payload, '$.update.sessionUpdate') = 'context_rolled_over'
               ) AS renewed
        FROM with_opener o
        LEFT JOIN events p ON p.session_id = o.session_id AND p.seq = o.opener_seq`,
      )
      .all(since) as Array<{
      session_id: string;
      occurred_at_ms: number;
      provider: string;
      stop_reason: string;
      inclusive_input_tokens: number | null;
      cached_read_tokens: number | null;
      cache_write_tokens: number | null;
      uncached_input_tokens: number | null;
      opener_payload: string | null;
      idle_gap_ms: number | null;
      renewed: number;
    }>;

    const rows: CacheFactInputRow[] = raw.map((r) => ({
      session_key: r.session_id,
      occurred_at_ms: r.occurred_at_ms,
      provider: r.provider,
      provider_version: "",
      model: null,
      stop_reason: r.stop_reason,
      prompt_origin: classifyOpenerOrigin(r.opener_payload),
      idle_gap_ms: r.idle_gap_ms,
      renewed: r.renewed === 1,
      usage_observed: r.inclusive_input_tokens !== null,
      inclusive_input_tokens: r.inclusive_input_tokens,
      cached_read_tokens: r.cached_read_tokens,
      cache_write_tokens: normalizeCacheWriteForProvider(r.provider, r.cache_write_tokens),
      uncached_input_tokens: r.uncached_input_tokens,
    }));
    return summarizeCacheTelemetryRows(rows, nowMs, days, "events_fallback", {
      projectionComplete: false,
    });
  } finally {
    if (inTx) {
      try {
        db.exec("ROLLBACK");
      } catch {
        // Ignore rollback errors on already closed handle.
      }
    }
    db.close();
  }
}
