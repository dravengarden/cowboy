import { Database } from "bun:sqlite";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { formatLogfmt, renderMarkdown, runPeriodicAudit } from "./periodic-audit.ts";
import { buildCatalog, digest, readObservations } from "./score.ts";

test("periodic audit aggregates catalog, routing, live ledger drift and harness upgrades", async () => {
  const root = mkdtempSync(join(tmpdir(), "cowboy-periodic-audit-"));
  try {
    mkdirSync(join(root, "models/observations"), { recursive: true });
    mkdirSync(join(root, "models/field"), { recursive: true });
    mkdirSync(join(root, "plugins/codex"), { recursive: true });

    const methodologyText = `schema = 1
efforts = ["medium", "max"]
[provenance]
independent = 1.0
third_party = 0.7
vendor = 0.25
internal = 1.0
[freshness]
half_life_days = 60
provisional_days = 14
[evidence]
min_lineages = 2
min_workload_coverage = 0.7
high_confidence_lineages = 3
high_confidence_max_age_days = 45
saturation_fraction = 0.9
min_spread = 0.03
uncertainty_floor = 0.05
[tasks.code_review]
benchmarks = ["rb"]
[workloads.session_default]
weights = { code_review = 1.0 }
cost_metric = "tokens_per_task"
noninferiority_margin = 0.05
value_floor = 0.5
tie_margin = 0.03
`;
    const targetsText = `schema = 1
[[targets]]
id = "gpt-6.2-sol:max"
vendor = "openai"
model = "gpt-6.2-sol"
effort = "max"
released = "2026-10-01"
provider = "codex"
preset = "sol-6-2-max"
values = { model = "gpt-6.2-sol", reasoning_effort = "max" }
`;
    writeFileSync(join(root, "models/methodology.toml"), methodologyText);
    writeFileSync(join(root, "models/targets.toml"), targetsText);
    writeFileSync(
      join(root, "models/observations/2026-11.jsonl"),
      `${JSON.stringify({
        id: "rb/1",
        target: "gpt-6.2-sol:max",
        benchmark: "rb",
        axis: "capability",
        metric: "recall",
        unit: "fraction",
        ceiling: 1,
        value: 0.85,
        measured_at: "2026-11-15",
        lineage: "l1",
        provenance: "independent",
        source: { url: "https://example.test", publisher: "ex", rights: "redistributable" },
      })}\n`,
    );
    writeFileSync(join(root, "models/field/2026-11-23.json"), JSON.stringify({ schema: 1 }));
    writeFileSync(
      join(root, "models/routing.json"),
      JSON.stringify({
        schema: 1,
        evidence: "2026-11-23",
        providers: {
          codex: {
            high: { auto: [{ family: "sol", model: "gpt-6.2-sol", efforts: ["max"] }] },
            routine: { auto: [{ family: "sol", model: "gpt-6.2-sol", efforts: ["max"] }] },
          },
        },
        costs: {
          "gpt-6.2-sol:max": {
            field: {
              median_tokens: 1_000_000,
              source: "models/field/2026-11-23.json (14-day ledger)",
            },
          },
        },
      }),
    );
    writeFileSync(
      join(root, "plugins/codex/provider.json"),
      JSON.stringify({
        id: "codex",
        version: "3.6.32",
        configuration_presets: [
          {
            id: "sol-6-2-max",
            is_default: true,
            values: { model: "gpt-6.2-sol", reasoning_effort: "max" },
          },
        ],
        runtime: {
          dependencies: [
            {
              id: "openai-codex",
              version: "0.171.0",
              source: "https://registry.npmjs.org/@openai/codex/-/codex-0.171.0.tgz",
              integrity: "sha512-ok",
              private: true,
            },
          ],
        },
      }),
    );

    const m = Bun.TOML.parse(methodologyText) as Parameters<typeof buildCatalog>[0];
    const t = (Bun.TOML.parse(targetsText) as { targets: Parameters<typeof buildCatalog>[1] })
      .targets;
    const repoObs = readObservations(join(root, "models/observations"), false);
    const localEmpty = { rows: [], count: 0, digest: digest(""), present: false };
    const cat = buildCatalog(m, t, repoObs, localEmpty, "2026-11-23", {
      methodology: digest(methodologyText),
      targets: digest(targetsText),
    });
    writeFileSync(join(root, "models/catalog.json"), JSON.stringify(cat, null, 2));

    // Create a minimal Controller SQLite ledger with 5 reviews showing +60% token drift.
    const dbPath = join(root, "cowboy.sqlite3");
    const db = new Database(dbPath);
    db.exec(`CREATE TABLE managed_agent_calls (
      state TEXT,
      document TEXT,
      request_document TEXT
    )`);
    const nowMs = Date.parse("2026-11-23T00:00:00Z");
    const insert = db.query("INSERT INTO managed_agent_calls VALUES (?, ?, ?)");
    for (let i = 0; i < 5; i++) {
      insert.run(
        "completed",
        JSON.stringify({
          provider: "codex",
          purpose: "review",
          selection: "auto",
          created_at_ms: nowMs - 3600_000,
          started_at_ms: nowMs - 3600_000,
          finished_at_ms: nowMs - 3300_000,
          configuration: { model: "gpt-6.2-sol", reasoning_effort: "max" },
          result: {
            usage: { total_tokens: 1_600_000, uncached_input_tokens: 100_000 },
            structured: { verdict: "approve", findings: [] },
          },
        }),
        JSON.stringify({ labels: { review_risk: "routine" } }),
      );
    }
    db.close();

    const report = await runPeriodicAudit({
      repoRoot: root,
      database: dbPath,
      asOf: "2026-11-23",
      nowMs,
      localObservations: join(root, "missing-local"),
      checkUpstream: true,
      fetchFn: async (url) => {
        if (url.endsWith("/0.171.0")) {
          return new Response(
            JSON.stringify({
              version: "0.171.0",
              dist: {
                integrity: "sha512-ok",
                tarball: "https://registry.npmjs.org/@openai/codex/-/codex-0.171.0.tgz",
              },
            }),
            { status: 200 },
          );
        }
        return new Response(
          JSON.stringify({ version: "0.172.0", dist: { integrity: "sha512-new" } }),
          { status: 200 },
        );
      },
    });

    assertEquals(report.action_needed, true);
    assertEquals(report.harness_inventory, [
      {
        provider_id: "codex",
        version: "3.6.32",
        default_preset: "sol-6-2-max",
        preset_count: 1,
        targets_covered: 1,
        managed_profiles: [],
        structured_output: false,
        routed_for_review: true,
        routed_for_analysis: false,
        runtime_pins: { "openai-codex": "0.171.0" },
      },
    ]);
    assertEquals(report.field_telemetry.available, true);
    if (report.field_telemetry.available) {
      assertEquals(report.field_telemetry.total_reviews, 5);
      assertEquals(report.field_telemetry.cost_drifts.length, 1);
      assertEquals(report.field_telemetry.cost_drifts[0]?.delta_ratio, 0.6);
    }
    assertEquals(report.upstream_dependencies.checked, true);
    if (report.upstream_dependencies.checked) {
      assertEquals(report.upstream_dependencies.upgrades_available, 1);
    }
    const md = renderMarkdown(report);
    assertEquals(md.includes("field-cost-drift: gpt-6.2-sol:max"), true);
    assertEquals(md.includes("harness-upgrade: codex/openai-codex 0.171.0 -> 0.172.0"), true);
    const log = formatLogfmt(report);
    assertEquals(log.includes("event_name=cowboy.model_intel.audit"), true);
    assertEquals(log.includes("action_needed=true"), true);
    assertEquals(log.includes("cost_drifts=1"), true);
    assertEquals(log.includes("upgrades_available=1"), true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("periodic audit reports quota pressure alerts and burn rates", async () => {
  const root = mkdtempSync(join(tmpdir(), "cowboy-periodic-quota-"));
  try {
    mkdirSync(join(root, "models/observations"), { recursive: true });
    mkdirSync(join(root, "models/field"), { recursive: true });
    mkdirSync(join(root, "plugins/codex"), { recursive: true });
    const methodologyText = `schema = 1
efforts = ["max"]
[provenance]
independent = 1.0
third_party = 0.7
vendor = 0.25
internal = 1.0
[freshness]
half_life_days = 60
provisional_days = 14
[evidence]
min_lineages = 2
min_workload_coverage = 0.7
high_confidence_lineages = 3
high_confidence_max_age_days = 45
saturation_fraction = 0.9
min_spread = 0.03
uncertainty_floor = 0.05
[tasks.code_review]
benchmarks = ["rb"]
[workloads.session_default]
weights = { code_review = 1.0 }
cost_metric = "tokens_per_task"
noninferiority_margin = 0.05
value_floor = 0.5
tie_margin = 0.03
`;
    const targetsText = `schema = 1
[[targets]]
id = "gpt-6.2-sol:max"
vendor = "openai"
model = "gpt-6.2-sol"
effort = "max"
provider = "codex"
preset = "sol-6-2-max"
values = { model = "gpt-6.2-sol", reasoning_effort = "max" }
`;
    writeFileSync(join(root, "models/methodology.toml"), methodologyText);
    writeFileSync(join(root, "models/targets.toml"), targetsText);
    writeFileSync(join(root, "models/observations/2026-11.jsonl"), "");
    const m = Bun.TOML.parse(methodologyText) as Parameters<typeof buildCatalog>[0];
    const t = (Bun.TOML.parse(targetsText) as { targets: Parameters<typeof buildCatalog>[1] })
      .targets;
    const empty = { rows: [], count: 0, digest: digest(""), present: false };
    const cat = buildCatalog(m, t, empty, empty, "2026-11-23", {
      methodology: digest(methodologyText),
      targets: digest(targetsText),
    });
    writeFileSync(join(root, "models/catalog.json"), JSON.stringify(cat));
    writeFileSync(
      join(root, "plugins/codex/provider.json"),
      JSON.stringify({
        id: "codex",
        version: "3.6.32",
        configuration_presets: [
          {
            id: "sol-6-2-max",
            is_default: true,
            values: { model: "gpt-6.2-sol", reasoning_effort: "max" },
          },
        ],
        runtime: {
          managed_profiles: { read_only_v1: { capabilities: ["structured_output"] } },
          dependencies: [],
        },
      }),
    );
    const nowMs = Date.parse("2026-11-23T12:00:00Z");
    const resetSec = Math.floor(Date.parse("2026-11-28T00:00:00Z") / 1000);
    const usagePath = join(root, "usage-named.json");
    writeFileSync(
      usagePath,
      JSON.stringify({
        schema: 1,
        entries: [
          {
            usage: {
              provider: "openai",
              status: "available",
              observed_at_ms: nowMs - 60_000,
              rate_limits: {
                rateLimitsByLimitId: {
                  codex: {
                    primary: { usedPercent: 85, windowDurationMins: 10080, resetsAt: resetSec },
                  },
                },
              },
            },
          },
        ],
      }),
    );
    const historyPath = join(root, "quota-history.jsonl");
    writeFileSync(
      historyPath,
      `${JSON.stringify({
        provider: "openai",
        limit_id: "codex",
        used_percent: 75,
        window_duration_mins: 10080,
        resets_at_ms: resetSec * 1000,
        observed_at_ms: nowMs - 24 * 3_600_000 - 60_000,
        observed_age_mins: 1,
        stale: false,
      })}
`,
    );
    const report = await runPeriodicAudit({
      repoRoot: root,
      usageFile: usagePath,
      quotaHistoryPath: historyPath,
      asOf: "2026-11-23",
      nowMs,
      localObservations: join(root, "missing-local"),
    });
    assertEquals(report.harness_inventory[0]?.structured_output, true);
    assertEquals(report.quota_telemetry.available, true);
    if (report.quota_telemetry.available) {
      assertEquals(report.quota_telemetry.pressure_alerts, ["openai/codex at 85% used"]);
      assertEquals(report.quota_telemetry.burns[0]?.burn_percent_per_day, 10);
    }
    assertEquals(report.reasons.includes("quota-pressure: openai/codex at 85% used"), true);
    assertEquals(formatLogfmt(report).includes("quota_pressure=1"), true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
