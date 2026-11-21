// Field cost and outcome of managed reviews, read from a Controller ledger.
//
// Usage (repository root, dev shell):
//   bun .agents/skills/model-intel/scripts/field.ts \
//     --database /var/lib/cowboy/cowboy.sqlite3 --days 14 --out models/field/2026-11-21.json
//
// Read-only: the database opens read-only. Only aggregates per
// (Provider, purpose, resolved model, effort) are written: no session, call,
// account, path or text. These are observational field data from a mixed
// workload, not a paired quality comparison; `score.ts` never scores them.

import { Database } from "bun:sqlite";
import { mkdirSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

export type Call = {
  provider: string;
  purpose: string;
  model: string;
  effort: string;
  risk: string;
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
        median_tokens: quantile(numbers((call) => call.tokens), 0.5),
        p75_tokens: quantile(numbers((call) => call.tokens), 0.75),
        median_uncached_tokens: quantile(numbers((call) => call.uncached), 0.5),
        median_minutes: quantile(numbers((call) => call.minutes), 0.5),
        mean_findings: findings.length
          ? Math.round((findings.reduce((a, b) => a + b, 0) / findings.length) * 10) / 10
          : null,
        verdicts: count(members.map((call) => call.verdict ?? "none")),
        risk: count(members.map((call) => call.risk)),
      };
    })
    .sort((a, b) => b.n - a.n);
}

function read(database: string, since: number): Call[] {
  const db = new Database(database, { readonly: true });
  try {
    const rows = db
      .query(
        `SELECT json_extract(document,'$.provider') AS provider,
                json_extract(document,'$.purpose') AS purpose,
                json_extract(document,'$.configuration.model') AS model,
                COALESCE(json_extract(document,'$.configuration.reasoning_effort'),
                         json_extract(document,'$.configuration.effort'), 'default') AS effort,
                COALESCE(json_extract(request_document,'$.labels.review_risk'), 'undeclared') AS risk,
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
           AND json_extract(document,'$.purpose') IN ('review','design_review')
           AND json_extract(document,'$.configuration.model') IS NOT NULL
           -- A reused receipt copies another run's outcome without running.
           AND json_extract(document,'$.result.reused_from') IS NULL
           AND json_extract(document,'$.created_at_ms') >= ?`,
      )
      .all(since) as Call[];
    return rows.map((row) => ({
      ...row,
      minutes: row.minutes === null ? null : Math.round(row.minutes * 10) / 10,
    }));
  } finally {
    db.close();
  }
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
  const report = {
    schema: 1,
    measured_at: new Date(now).toISOString().slice(0, 10),
    window_days: days,
    source: "Hawk Controller managed_agent_calls (read-only), completed reviews and design reviews",
    caveat: "Observational mixed workload: inputs differ between groups, so this compares cost, not quality.",
    groups: aggregate(read(database, now - days * 86_400_000)),
  };
  const text = `${JSON.stringify(report, null, 2)}\n`;
  if (out) {
    const target = resolve(out);
    mkdirSync(dirname(target), { recursive: true });
    // Never overwrite the ledger (or its WAL/SHM files) through a typo or link.
    const ledger = realpathSync(database);
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
    const staged = `${target}.tmp-${process.pid}`;
    writeFileSync(staged, text);
    renameSync(staged, target);
  } else {
    process.stdout.write(text);
  }
}
