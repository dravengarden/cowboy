import { test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { assertEquals, assertThrows } from "../../../../tools/test-assert.ts";
import {
  audit,
  buildCatalog,
  type Candidate,
  type Cell,
  canonical,
  type Inputs,
  type Methodology,
  normalize,
  type Observation,
  readObservations,
  scoreCell,
  type Target,
  tiers,
} from "./score.ts";

const wl = {
  weights: { code_review: 1 },
  cost_metric: "tokens_per_task",
  noninferiority_margin: 0.05,
  value_floor: 0.5,
  tie_margin: 0.03,
};
const m: Methodology = {
  schema: 1,
  provenance: { independent: 1, third_party: 0.7, vendor: 0.25, internal: 1 },
  freshness: { half_life_days: 60, provisional_days: 14 },
  evidence: {
    min_lineages: 2,
    min_workload_coverage: 0.7,
    high_confidence_lineages: 3,
    high_confidence_max_age_days: 45,
    saturation_fraction: 0.9,
    min_spread: 0.03,
    uncertainty_floor: 0.05,
  },
  tasks: { code_review: { benchmarks: ["rb", "pilot"] } },
  workloads: { session_default: wl },
};

const target = (id: string, released?: string): Target => ({
  id,
  vendor: "v",
  model: id,
  effort: "max",
  released,
  provider: "p",
  preset: id,
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
const inputs = (rows: Observation[]): Inputs => ({
  rows,
  count: rows.length,
  digest: "sha256:x",
  present: true,
});
const cell = (mean: number): Cell => ({ status: "scored", mean, uncertainty: 0.05, lineages: 2 });
const cand = (preset: string, mean: number, costs: Record<string, number>): Candidate => ({
  preset,
  cells: { code_review: cell(mean) },
  costs: new Map(Object.entries(costs)),
});

test("a mirrored lineage counts once and cannot set the scale", () => {
  // A leaderboard and a lower-trust mirror with an inflated copy are one run;
  // the mirror must neither add a lineage nor saturate the group.
  const rows = canonical(
    [
      obs({ target: "a", value: 0.7, lineage: "board" }),
      obs({ target: "a", value: 0.95, lineage: "board", provenance: "third_party" }),
      obs({ target: "b", value: 0.5, lineage: "board" }),
    ],
    m,
  );
  const { values, dropped } = normalize(rows, m);
  assertEquals(dropped, []);
  const mine = rows.filter((o) => o.target === "a");
  assertEquals(scoreCell(mine, values, m, "2026-11-18"), { status: "insufficient", lineages: 1 });
});

test("saturated groups are dropped and lower-is-better bounds are oriented", () => {
  const sat = normalize([obs({ target: "a", value: 0.95 }), obs({ target: "b", value: 0.6 })], m);
  assertEquals(sat.values.size, 0);
  const err = obs({ target: "a", value: 0.2, higher_is_better: false, metric: "error" });
  assertEquals(normalize([err], m).values.get(err.id), 0.8);
});

test("unverified and superseded rows never feed scores", () => {
  const rows = canonical(
    [
      obs({ target: "a", value: 0.7, validity: "unverified" }),
      obs({ target: "a", value: 0.7, validity: "superseded" }),
    ],
    m,
  );
  assertEquals(rows, []);
});

test("strong skips leaders within the tie margin", () => {
  const t = tiers(
    [cand("big", 0.8, { g: 10 }), cand("near", 0.79, { g: 3 }), cand("mid", 0.7, { g: 1 })],
    wl,
    0.7,
  );
  assertEquals([t.flagship, t.tied_with_flagship, t.strong], ["big", ["near"], "mid"]);
  assertEquals([t.workhorse, t.value], ["near", "mid"]);
});

test("cost tiers need paired comparisons with every candidate", () => {
  // `cheap` was only measured on another workload; it must not win by default.
  const t = tiers([cand("a", 0.8, { g1: 5 }), cand("cheap", 0.79, { g2: 1 })], wl, 0.7);
  assertEquals([t.workhorse, t.value], [undefined, undefined]);
});

test("candidates are ranked only on their common tasks", () => {
  const w = { ...wl, weights: { code_review: 0.5, reasoning: 0.5 } };
  const a: Candidate = { preset: "a", cells: { code_review: cell(0.9) }, costs: new Map() };
  const b: Candidate = { preset: "b", cells: { reasoning: cell(0.9) }, costs: new Map() };
  const t = tiers([a, b], w, 0.7);
  assertEquals(t.flagship, undefined);
  assertEquals(t.notes, ["candidates do not share enough scored tasks to be ranked"]);
});

const pair = (t: string, v: number) => [
  obs({ target: t, value: v }),
  obs({ target: t, value: v, benchmark: "pilot" }),
];

test("provisional models are scored but never tiered", () => {
  const c = buildCatalog(
    m,
    [target("new", "2026-11-17"), target("old", "2026-09-01"), target("older", "2026-08-01")],
    inputs([...pair("new", 0.8), ...pair("old", 0.6), ...pair("older", 0.4)]),
    inputs([]),
    "2026-11-18",
  );
  assertEquals(c.targets[0].tasks.code_review.status, "scored");
  assertEquals(
    [c.tiers.p.session_default.flagship, c.tiers.p.session_default.strong],
    ["old", "older"],
  );
  assertEquals(c.tiers.p.session_default.notes.at(-1), "provisional, not ranked: new");
});

test("a single rankable candidate gets no tier", () => {
  const t = tiers([cand("only", 0.8, { g: 1 })], wl, 0.7);
  assertEquals(t.flagship, undefined);
  assertEquals(t.notes, ["only only is rankable; tiers need two candidates"]);
});

test("a cost tier needs cost evidence even for one qualifying candidate", () => {
  const t = tiers([cand("a", 0.8, {}), cand("b", 0.3, {})], wl, 0.7);
  assertEquals([t.flagship, t.workhorse, t.value], ["a", undefined, undefined]);
});

test("unmapped and post-snapshot rows do not shape mapped scores", () => {
  const typo = obs({ target: "typo", value: 0.95 });
  const late = obs({ target: "a", value: 0.95, measured_at: "2026-12-01" });
  const c = buildCatalog(
    m,
    [target("a", "2026-01-01"), target("b", "2026-01-01")],
    inputs([...pair("a", 0.7), ...pair("b", 0.5), typo, late]),
    inputs([]),
    "2026-11-18",
  );
  assertEquals(c.dropped_groups, []);
  assertEquals(c.unmapped_targets, ["typo"]);
  assertEquals(c.inputs.excluded_after_snapshot, 1);
  assertEquals(c.targets[0].tasks.code_review.mean, 0.7);
});

test("repository observations must be redistributable", () => {
  const dir = mkdtempSync(join(tmpdir(), "model-intel-"));
  const row = obs({ target: "a", value: 0.5 });
  row.source = { ...row.source, rights: "reference_only" };
  writeFileSync(join(dir, "x.jsonl"), `${JSON.stringify(row)}\n`);
  assertThrows(() => readObservations(dir, false));
  assertEquals(readObservations(dir, true).count, 1);
});

test("audit asks for a refresh when a provisional window ends", () => {
  const c = buildCatalog(m, [target("new", "2026-11-17")], inputs([]), inputs([]), "2026-11-18");
  const findings = audit(
    c,
    [target("new", "2026-11-17")],
    {},
    { repository: "sha256:x", local: "sha256:x" },
    "2026-12-02",
  );
  assertEquals(findings, [
    "provisional window ended for new; refresh",
    "p/new: target has no signed preset",
  ]);
});

test("audit covers untargeted Providers, TOML drift and the leading card order", () => {
  const targets = [target("a", "2026-01-01"), target("b", "2026-01-01")];
  const c = buildCatalog(m, targets, inputs([]), inputs([]), "2026-10-01");
  c.recommendations.p = { order: ["a", "b"], default: "b" };
  const drifted = [{ ...targets[0], values: { model: "changed" } }, targets[1]];
  const findings = audit(
    c,
    drifted,
    {
      p: {
        configuration_presets: [
          { id: "x", values: {} },
          { id: "a", values: { model: "changed" } },
          { id: "b", is_default: true, values: { model: "b" } },
        ],
      },
      grok: { configuration_presets: [{ id: "grok-high", values: {} }] },
    },
    { repository: "sha256:x", local: null, methodology: "changed", targets: "" },
    "2026-11-18",
  );
  assertEquals(findings, [
    "catalog is 48 days old",
    "methodology.toml changed since the catalog was generated",
    "local reference-only observations unavailable; local inputs not verified",
    "targets.toml bindings changed since the catalog was generated",
    "grok/grok-high: signed preset has no evaluation target",
    "p/x: signed preset has no evaluation target",
    "p: evidence order a > b differs from signed leading cards x > a",
  ]);
});
