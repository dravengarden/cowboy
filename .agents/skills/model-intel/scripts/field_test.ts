import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { aggregate, type Call } from "./field.ts";

const call = (overrides: Partial<Call>): Call => ({
  provider: "codex",
  purpose: "review",
  model: "gpt-6.2-sol",
  effort: "max",
  risk: "routine",
  tokens: 100,
  uncached: 10,
  minutes: 1,
  verdict: "approve",
  findings: 0,
  ...overrides,
});

test("field groups by provider, purpose and resolved target and ignores missing usage", () => {
  const groups = aggregate([
    call({ tokens: 100, findings: 2 }),
    call({ tokens: 300, findings: 4, verdict: "needs-attention" }),
    call({ tokens: null, findings: null, verdict: null, risk: "undeclared" }),
    call({ model: "gpt-6.2-astra", tokens: 900 }),
  ]);
  assertEquals(
    groups.map((group) => [group.target, group.n]),
    [
      ["gpt-6.2-sol:max", 3],
      ["gpt-6.2-astra:max", 1],
    ],
  );
  const sol = groups[0]!;
  assertEquals(sol.with_usage, 2);
  assertEquals(sol.median_tokens, 200);
  assertEquals(sol.mean_findings, 3);
  assertEquals(sol.verdicts, { approve: 1, "needs-attention": 1, none: 1 });
  assertEquals(sol.risk, { routine: 2, undeclared: 1 });
});

import { detectCostDrifts, summarizeTelemetry } from "./field.ts";
import type { RoutingTable } from "./score.ts";

test("summarizeTelemetry splits review and analysis calls and detects cost drift", () => {
  const routing: RoutingTable = {
    schema: 1,
    evidence: "2026-11-21",
    providers: {},
    costs: {
      "gpt-6.2-sol:max": {
        field: { median_tokens: 1000, n: 10 },
      },
    },
  };
  const calls = [
    ...Array.from({ length: 5 }, () => call({ tokens: 1500, selection: "auto", risk: "routine" })),
    call({
      provider: "antigravity",
      purpose: "analysis",
      model: "gemini-4-hydrogen",
      effort: "high",
      tokens: 90,
      selection: "explicit",
      risk: "undeclared",
      verdict: null,
      findings: null,
    }),
  ];
  const summary = summarizeTelemetry(calls, routing, { minSamples: 5, threshold: 0.3 });
  assertEquals(summary.review_groups.length, 1);
  assertEquals(summary.analysis_groups.length, 1);
  assertEquals(summary.analysis_groups[0]?.target, "gemini-4-hydrogen:high");
  assertEquals(summary.review_selection, { auto: 5 });
  assertEquals(summary.cost_drifts, [
    {
      target: "gpt-6.2-sol:max",
      recorded_median_tokens: 1000,
      live_median_tokens: 1500,
      delta_ratio: 0.5,
      n: 5,
    },
  ]);
  // Below minSamples produces no drift alert.
  assertEquals(
    detectCostDrifts(summary.review_groups, routing, { minSamples: 10, threshold: 0.3 }),
    [],
  );
});

import { Database } from "bun:sqlite";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { computeQuotaBurn, readDeepSeekGatewayStats, readQuotaSnapshots } from "./field.ts";

test("readQuotaSnapshots anonymizes windows and computeQuotaBurn correlates calls", () => {
  const dir = mkdtempSync(join(tmpdir(), "cowboy-quota-test-"));
  try {
    const usagePath = join(dir, "usage-named.json");
    const nowMs = Date.parse("2026-11-23T12:00:00Z");
    const resetSec = Math.floor(Date.parse("2026-11-28T00:00:00Z") / 1000);
    writeFileSync(
      usagePath,
      JSON.stringify({
        schema: 1,
        entries: [
          {
            owner: "secret-owner",
            account: "secret-uuid",
            usage: {
              provider: "openai",
              status: "available",
              observed_at_ms: nowMs - 5 * 60_000,
              rate_limits: {
                rateLimitsByLimitId: {
                  codex: {
                    primary: { usedPercent: 52, windowDurationMins: 10080, resetsAt: resetSec },
                  },
                },
              },
            },
          },
        ],
      }),
    );
    const current = readQuotaSnapshots(usagePath, nowMs);
    assertEquals(current.length, 1);
    assertEquals(current[0]?.provider, "openai");
    assertEquals(current[0]?.used_percent, 52);
    assertEquals(current[0]?.stale, false);
    const prev = [
      {
        ...current[0]!,
        used_percent: 40,
        observed_at_ms: current[0]!.observed_at_ms - 12 * 3_600_000,
      },
    ];
    const burns = computeQuotaBurn(prev, current, () => 24);
    assertEquals(burns, [
      {
        provider: "openai",
        limit_id: "codex",
        delta_used_percent: 12,
        elapsed_hours: 12,
        burn_percent_per_day: 24,
        calls_in_window: 24,
        estimated_percent_per_call: 0.5,
      },
    ]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("readDeepSeekGatewayStats aggregates DeepSeek requests and cache hit ratio", () => {
  const dir = mkdtempSync(join(tmpdir(), "cowboy-ds-test-"));
  try {
    const dbPath = join(dir, "cowboy.sqlite3");
    const db = new Database(dbPath);
    db.exec(`CREATE TABLE provider_usage_events (
      provider TEXT, agent TEXT, model_family TEXT, occurred_at_ms INTEGER,
      input_tokens INTEGER, output_tokens INTEGER, cache_hit_tokens INTEGER, cache_miss_tokens INTEGER
    )`);
    const now = Date.parse("2026-11-23T00:00:00Z");
    const ins = db.query("INSERT INTO provider_usage_events VALUES (?,?,?,?,?,?,?,?)");
    ins.run("deepseek", "claude", "flash", now - 1000, 900, 100, 800, 200);
    ins.run("deepseek", "claude", "flash", now - 2000, 1900, 100, 1600, 400);
    db.close();
    const stats = readDeepSeekGatewayStats(dbPath, now - 10_000);
    assertEquals(stats, [
      {
        agent: "claude",
        model_family: "flash",
        requests: 2,
        median_total_tokens: 1500,
        cache_hit_ratio: 0.8,
      },
    ]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
