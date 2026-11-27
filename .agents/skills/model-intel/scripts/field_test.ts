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
import {
  computeQuotaBurn,
  pruneOlderSnapshots,
  readDeepSeekGatewayStats,
  readQuotaSnapshots,
} from "./field.ts";

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

test("pruneOlderSnapshots removes older dated snapshots and keeps latest and non-dated files", () => {
  const dir = mkdtempSync(join(tmpdir(), "cowboy-prune-test-"));
  try {
    writeFileSync(join(dir, "2026-11-21.json"), "{}");
    writeFileSync(join(dir, "2026-11-23.json"), "{}");
    writeFileSync(join(dir, "notes.txt"), "keep");
    const removed = pruneOlderSnapshots(join(dir, "2026-11-23.json"));
    assertEquals(removed, ["2026-11-21.json"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

import { readdirSync, readFileSync } from "node:fs";
import { readCacheTelemetryAudit, summarizeCacheTelemetryRows } from "./field.ts";

test("summarizeCacheTelemetryRows detects version-cohort cache & cache-write drops even when an older healthy version dominates", () => {
  const nowMs = Date.parse("2026-11-27T00:00:00Z");
  const rows = [];
  // Healthy older v1.0.0 dominates the window (40 turns, 100% cache + write coverage).
  for (let i = 0; i < 40; i++) {
    rows.push({
      session_key: `sess-old-${i % 4}`,
      occurred_at_ms: nowMs - 8 * 86_400_000 + i * 60_000,
      provider: "claude-code",
      provider_version: "1.0.0",
      model: "claude-opus-4-6",
      stop_reason: "end_turn",
      prompt_origin: "human",
      idle_gap_ms: 12 * 60_000,
      renewed: false,
      usage_observed: true,
      inclusive_input_tokens: 100_000,
      cached_read_tokens: 80_000,
      cache_write_tokens: 18_000,
      uncached_input_tokens: 20_000,
    });
  }
  // Newly deployed v1.1.0 has only 12 turns (minority of volume) and loses cache_write_tokens while keeping cached_read.
  for (let i = 0; i < 12; i++) {
    rows.push({
      session_key: `sess-new-${i % 2}`,
      occurred_at_ms: nowMs - 1 * 86_400_000 + i * 60_000,
      provider: "claude-code",
      provider_version: "1.1.0",
      model: "claude-opus-4-6",
      stop_reason: "end_turn",
      prompt_origin: "human",
      idle_gap_ms: 45 * 60_000,
      renewed: true,
      usage_observed: true,
      inclusive_input_tokens: 50_000,
      cached_read_tokens: 40_000,
      cache_write_tokens: null,
      uncached_input_tokens: 10_000,
    });
  }
  const audit = summarizeCacheTelemetryRows(rows, nowMs, 14, "turn_cache_facts");
  assertEquals(audit.coverage_regressions.length, 1);
  assertEquals(audit.coverage_regressions[0]?.comparison_kind, "version_cohort");
  assertEquals(audit.coverage_regressions[0]?.metric, "cache_write_coverage");
  assertEquals(audit.coverage_regressions[0]?.previous_version, "1.0.0");
  assertEquals(audit.coverage_regressions[0]?.current_version, "1.1.0");
  assertEquals(audit.coverage_regressions[0]?.drop_ratio, 1);
  assertEquals(audit.shadow_ttl_curves.length, 2);
  assertEquals(audit.shadow_ttl_curves[0]?.gap_bucket, "10-20m");
  assertEquals(audit.shadow_ttl_curves[0]?.distinct_sessions, 4);
  assertEquals(audit.shadow_ttl_curves[1]?.gap_bucket, "30-60m");
  assertEquals(audit.shadow_ttl_curves[1]?.renewed, true);
});

test("readCacheTelemetryAudit falls back to events when backfill is incomplete and never treats a human steer as opener", () => {
  const dir = mkdtempSync(join(tmpdir(), "cowboy-cache-audit-test-"));
  try {
    const dbPath = join(dir, "cowboy.sqlite3");
    const db = new Database(dbPath);
    db.exec(`CREATE TABLE turn_cache_facts (
      session_id TEXT, turn_end_seq INTEGER, occurred_at_ms INTEGER,
      provider TEXT, provider_version TEXT, model TEXT, stop_reason TEXT,
      prompt_origin TEXT, idle_gap_ms INTEGER, renewed INTEGER, usage_observed INTEGER,
      inclusive_input_tokens INTEGER, cached_read_tokens INTEGER, cache_write_tokens INTEGER
    );
    CREATE TABLE turn_cache_backfill_state (
      slot INTEGER PRIMARY KEY, completed_at_ms INTEGER
    );
    INSERT INTO turn_cache_backfill_state VALUES (1, NULL);
    CREATE TABLE sessions (
      id TEXT PRIMARY KEY, provider TEXT, provider_version TEXT,
      last_usage TEXT, config_preferences TEXT
    );
    CREATE TABLE events (
      session_id TEXT, seq INTEGER, ts_ms INTEGER, payload TEXT
    );`);
    const now = Date.parse("2026-11-27T00:00:00Z");
    db.query("INSERT INTO sessions VALUES (?,?,?,?,?)").run(
      "s1", "codex", "3.6.32", null, JSON.stringify({ model: "gpt-6.2-sol" }),
    );
    const ins = db.query("INSERT INTO events VALUES (?,?,?,?)");
    // Turn 1
    ins.run("s1", 1, now - 600_000, JSON.stringify({ kind: "turn_end", stop_reason: "EndTurn", usage: { input_tokens: 1000, cached_input_tokens: 800, uncached_input_tokens: 200 } }));
    // Turn 2: opened by automation (agent), later steered by a human -> must NOT become a human TTL bucket!
    ins.run("s1", 2, now - 300_000, JSON.stringify({ kind: "update", update: { sessionUpdate: "user_message_chunk", promptOrigin: { actor: "agent" }, content: { text: "notice" } } }));
    ins.run("s1", 3, now - 100_000, JSON.stringify({ kind: "update", update: { sessionUpdate: "user_message_chunk", promptOrigin: { actor: "human" }, _meta: { cowboyDelivery: "current_turn" }, content: { text: "steer" } } }));
    ins.run("s1", 4, now - 50_000, JSON.stringify({ kind: "turn_end", stop_reason: "EndTurn", usage: { input_tokens: 2000, cached_input_tokens: 1800, uncached_input_tokens: 200 } }));
    db.close();

    const audit = readCacheTelemetryAudit(dbPath, 14, now);
    assertEquals(audit.source, "events_fallback");
    assertEquals(audit.projection_complete, false);
    assertEquals(audit.providers[0]?.turns, 2);
    // Automation-opened turn steered by human must not enter human shadow TTL curves.
    assertEquals(audit.shadow_ttl_curves.length, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("models/cache-and-telemetry.json covers every first-party agent provider", () => {
  const repoRoot = join(import.meta.dir, "../../../..");
  const inventory = JSON.parse(
    readFileSync(join(repoRoot, "models/cache-and-telemetry.json"), "utf8"),
  ) as { authority: string; providers: Record<string, unknown> };
  assertEquals(inventory.authority, "advisory");
  const pluginsDir = join(repoRoot, "plugins");
  for (const entry of readdirSync(pluginsDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const providerPath = join(pluginsDir, entry.name, "provider.json");
    try {
      const manifest = JSON.parse(readFileSync(providerPath, "utf8")) as { id: string };
      assertEquals(
        Object.hasOwn(inventory.providers, manifest.id),
        true,
        `models/cache-and-telemetry.json missing provider ${manifest.id}`,
      );
    } catch {
      // Non-agent plugin directory without provider.json.
    }
  }
});
