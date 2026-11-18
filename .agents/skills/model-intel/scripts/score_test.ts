import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  audit,
  buildCatalog,
  type Methodology,
  normalize,
  type Observation,
  scoreCell,
  type Target,
  tiers,
} from "./score.ts";

const m: Methodology = {
  schema: 1,
  provenance: { independent: 1, third_party: 0.7, vendor: 0.25, internal: 1 },
  freshness: { half_life_days: 60, provisional_days: 14 },
  evidence: {
    min_lineages: 2,
    high_confidence_lineages: 3,
    high_confidence_max_age_days: 45,
    saturation_fraction: 0.9,
    min_spread: 0.03,
    uncertainty_floor: 0.05,
  },
  tasks: { code_review: { benchmarks: ["rb", "pilot"] } },
  workloads: {
    session_default: {
      weights: { code_review: 1 },
      noninferiority_margin: 0.05,
      value_floor: 0.5,
      tie_margin: 0.03,
    },
  },
};

const target = (id: string, preset: string, released?: string): Target => ({
  id,
  vendor: "v",
  model: id,
  effort: "max",
  released,
  provider: "p",
  preset,
  values: { model: id },
});

let seq = 0;
const obs = (o: Partial<Observation> & Pick<Observation, "target" | "value">): Observation => ({
  id: `o${seq++}`,
  benchmark: "rb",
  axis: "capability",
  metric: "recall",
  unit: "fraction",
  ceiling: 1,
  measured_at: "2026-11-01",
  lineage: `l${seq}`,
  provenance: "independent",
  source: { url: "https://example.test", publisher: "x", rights: "redistributable" },
  ...o,
});

test("one lineage republished by mirrors counts once", () => {
  // A leaderboard and its aggregator mirror are one measurement, not two.
  const a = [
    obs({ target: "a", value: 0.7, lineage: "board" }),
    obs({ target: "a", value: 0.7, lineage: "board", provenance: "third_party" }),
  ];
  const { values } = normalize(a, m);
  assertEquals(scoreCell(a, values, m, "2026-11-18"), { status: "insufficient", lineages: 1 });
});

test("saturated benchmark groups are dropped instead of scored", () => {
  const a = [obs({ target: "a", value: 0.95 }), obs({ target: "b", value: 0.6 })];
  const { values, dropped } = normalize(a, m);
  assertEquals(values.size, 0);
  assertEquals(dropped.length, 1);
});

test("a vendor claim weighs less than an independent run", () => {
  const a = [
    obs({ target: "a", value: 0.8, lineage: "vendor", provenance: "vendor" }),
    obs({ target: "a", value: 0.4, lineage: "indep", benchmark: "pilot" }),
  ];
  const { values } = normalize(a, m);
  const cell = scoreCell(a, values, m, "2026-11-01");
  assertEquals(cell.status, "scored");
  assertEquals((cell.mean as number) < 0.6, true);
});

test("workhorse is the cheapest non-inferior target and becomes the default", () => {
  const scored = [
    { target: target("big", "big"), mean: 0.8, uncertainty: 0.05, cost: 10 },
    { target: target("mid", "mid"), mean: 0.78, uncertainty: 0.05, cost: 2 },
    { target: target("small", "small"), mean: 0.55, uncertainty: 0.05, cost: 1 },
  ];
  const t = tiers(scored, m.workloads.session_default);
  assertEquals([t.flagship, t.strong, t.workhorse, t.value], ["big", "mid", "mid", "small"]);
  assertEquals(t.strong_tie, true);
});

test("unknown cost leaves workhorse undetermined rather than guessing", () => {
  const t = tiers(
    [{ target: target("a", "a"), mean: 0.8, uncertainty: 0.05, cost: null }],
    m.workloads.session_default,
  );
  assertEquals(t.workhorse, undefined);
  assertEquals(t.notes.length, 2);
});

test("provisional models are tiered only on non-vendor evidence", () => {
  const targets = [target("new", "new", "2026-11-17"), target("old", "old", "2026-09-01")];
  const o = [
    obs({ target: "new", value: 0.8, provenance: "vendor" }),
    obs({ target: "new", value: 0.8, benchmark: "pilot" }),
    obs({ target: "old", value: 0.6 }),
    obs({ target: "old", value: 0.6, benchmark: "pilot" }),
    obs({ target: "old", axis: "cost", metric: "usd", unit: "usd", value: 1, ceiling: undefined }),
  ];
  const c = buildCatalog(m, targets, o, "2026-11-18");
  assertEquals(c.tiers.p.session_default.flagship, "old");
  assertEquals(c.recommendations.p, { order: ["old"], default: "old" });
});

test("audit reports drift between evidence and signed presets", () => {
  const targets = [target("a", "a", "2026-01-01")];
  const c = buildCatalog(m, targets, [], "2026-10-01");
  const findings = audit(
    c,
    {
      p: {
        configuration_presets: [
          { id: "a", is_default: true, values: { model: "other" } },
          { id: "x", values: {} },
        ],
      },
    },
    "2026-11-18",
  );
  assertEquals(findings, [
    "catalog is 48 days old",
    "p/a: target values differ from the signed preset",
    "p/x: signed preset has no evaluation target",
  ]);
});
