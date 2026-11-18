// Deterministic model evidence scorer.
//
// Reads models/methodology.toml, models/targets.toml and observation JSONL
// files, then writes models/catalog.json and models/summary.md. `--audit`
// compares the result with the signed Provider presets without writing.
// Judgment (which sources to trust, what a disagreement means) belongs to the
// skill; every number here is a pure function of the versioned inputs.

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export type Provenance = "independent" | "third_party" | "vendor" | "internal";
export type Observation = {
  id: string;
  target: string;
  benchmark: string;
  benchmark_version?: string;
  harness?: string;
  axis: "capability" | "cost" | "preference" | "adoption";
  metric: string;
  unit: string;
  ceiling?: number;
  higher_is_better?: boolean;
  value: number;
  n?: number;
  measured_at: string;
  lineage: string;
  provenance: Provenance;
  source: {
    url: string;
    publisher: string;
    rights: "redistributable" | "reference_only";
    retrieved_at?: string;
  };
};
export type Target = {
  id: string;
  vendor: string;
  model: string;
  effort: string;
  released?: string;
  provider?: string;
  preset?: string;
  values?: Record<string, string>;
};
type Workload = {
  weights: Record<string, number>;
  noninferiority_margin: number;
  value_floor: number;
  tie_margin: number;
};
export type Methodology = {
  schema: number;
  provenance: Record<Provenance, number>;
  freshness: { half_life_days: number; provisional_days: number };
  evidence: {
    min_lineages: number;
    high_confidence_lineages: number;
    high_confidence_max_age_days: number;
    saturation_fraction: number;
    min_spread: number;
    uncertainty_floor: number;
  };
  tasks: Record<string, { benchmarks: string[] }>;
  workloads: Record<string, Workload>;
};
export type Cell = {
  status: "scored" | "insufficient";
  mean?: number;
  uncertainty?: number;
  lineages: number;
  independent_lineages?: number;
  max_age_days?: number;
  confidence?: "high" | "medium" | "low";
};
type Scored = { target: Target; mean: number; uncertainty: number; cost: number | null };
export type Tiers = {
  flagship?: string;
  strong?: string;
  strong_tie?: boolean;
  workhorse?: string;
  value?: string;
  notes: string[];
};

const DAY_MS = 86_400_000;
const round = (x: number) => Math.round(x * 1000) / 1000;
const days = (from: string, to: string) => (Date.parse(to) - Date.parse(from)) / DAY_MS;

export function groupKey(o: Observation): string {
  return [o.benchmark, o.benchmark_version ?? "", o.harness ?? "", o.metric].join("|");
}

// Normalize capability observations to [0, 1]. Metrics with a ceiling use the
// absolute fraction; others use min-max within their group (>= 3 targets).
// Saturated or flat groups are dropped and reported.
export function normalize(
  obs: Observation[],
  m: Methodology,
): { values: Map<string, number>; dropped: string[] } {
  const groups = new Map<string, Observation[]>();
  for (const o of obs.filter((o) => o.axis === "capability")) {
    const k = groupKey(o);
    groups.set(k, [...(groups.get(k) ?? []), o]);
  }
  const values = new Map<string, number>();
  const dropped: string[] = [];
  for (const [k, group] of [...groups].sort(([a], [b]) => a.localeCompare(b))) {
    const sign = (o: Observation) => (o.higher_is_better === false ? -o.value : o.value);
    const xs = group.map(sign);
    const hi = Math.max(...xs);
    const lo = Math.min(...xs);
    const ceiling = group[0].ceiling;
    if (ceiling !== undefined && group[0].higher_is_better !== false) {
      const spread = (hi - lo) / ceiling;
      if (
        hi / ceiling > m.evidence.saturation_fraction ||
        (new Set(group.map((o) => o.target)).size > 1 && spread < m.evidence.min_spread)
      ) {
        dropped.push(`${k}: saturated or flat`);
        continue;
      }
      for (const o of group) values.set(o.id, o.value / ceiling);
      continue;
    }
    if (new Set(group.map((o) => o.target)).size < 3 || hi - lo === 0) {
      dropped.push(`${k}: fewer than 3 targets for relative scale`);
      continue;
    }
    for (const o of group) values.set(o.id, (sign(o) - lo) / (hi - lo));
  }
  return { values, dropped };
}

// One value per lineage: the most trusted, then the newest observation.
function lineageValues(
  obs: Observation[],
  values: Map<string, number>,
  m: Methodology,
  asOf: string,
) {
  const byLineage = new Map<string, Observation>();
  for (const o of obs) {
    if (!values.has(o.id)) continue;
    const prev = byLineage.get(o.lineage);
    const better =
      !prev ||
      m.provenance[o.provenance] > m.provenance[prev.provenance] ||
      (m.provenance[o.provenance] === m.provenance[prev.provenance] &&
        o.measured_at > prev.measured_at);
    if (better) byLineage.set(o.lineage, o);
  }
  return [...byLineage.values()].map((o) => {
    const independent = o.provenance !== "vendor";
    const age = Math.max(0, days(o.measured_at, asOf));
    const weight = m.provenance[o.provenance] * 0.5 ** (age / m.freshness.half_life_days);
    return { value: values.get(o.id) as number, weight, age, independent };
  });
}

export function scoreCell(
  obs: Observation[],
  values: Map<string, number>,
  m: Methodology,
  asOf: string,
): Cell {
  const ls = lineageValues(obs, values, m, asOf);
  if (ls.length < m.evidence.min_lineages) return { status: "insufficient", lineages: ls.length };
  const w = ls.reduce((s, l) => s + l.weight, 0);
  const mean = ls.reduce((s, l) => s + l.value * l.weight, 0) / w;
  const variance = ls.reduce((s, l) => s + l.weight * (l.value - mean) ** 2, 0) / w;
  const uncertainty = Math.max(m.evidence.uncertainty_floor, Math.sqrt(variance / ls.length));
  const maxAge = Math.min(...ls.map((l) => l.age));
  const confidence =
    ls.length >= m.evidence.high_confidence_lineages &&
    maxAge <= m.evidence.high_confidence_max_age_days
      ? "high"
      : ls.length >= m.evidence.min_lineages
        ? "medium"
        : "low";
  return {
    status: "scored",
    mean: round(mean),
    uncertainty: round(uncertainty),
    lineages: ls.length,
    independent_lineages: ls.filter((l) => l.independent).length,
    max_age_days: Math.round(maxAge),
    confidence,
  };
}

export function workloadScore(
  cells: Record<string, Cell>,
  wl: Workload,
): Cell & { coverage: number } {
  let covered = 0;
  let mean = 0;
  let variance = 0;
  for (const [task, weight] of Object.entries(wl.weights)) {
    const c = cells[task];
    if (c?.status !== "scored") continue;
    covered += weight;
    mean += weight * (c.mean as number);
    variance += weight ** 2 * (c.uncertainty as number) ** 2;
  }
  if (covered < 0.7) return { status: "insufficient", lineages: 0, coverage: round(covered) };
  return {
    status: "scored",
    mean: round(mean / covered),
    uncertainty: round(Math.sqrt(variance) / covered),
    lineages: 0,
    coverage: round(covered),
  };
}

// Relative cost: per cost group, value / cheapest value in the group; a
// target's index is the geometric mean over the groups it appears in.
export function costIndex(obs: Observation[]): Map<string, number> {
  const groups = new Map<string, Observation[]>();
  for (const o of obs.filter((o) => o.axis === "cost" && o.value > 0)) {
    groups.set(groupKey(o), [...(groups.get(groupKey(o)) ?? []), o]);
  }
  const logs = new Map<string, number[]>();
  for (const group of groups.values()) {
    const min = Math.min(...group.map((o) => o.value));
    for (const o of group)
      logs.set(o.target, [...(logs.get(o.target) ?? []), Math.log(o.value / min)]);
  }
  return new Map(
    [...logs].map(([t, xs]) => [t, round(Math.exp(xs.reduce((a, b) => a + b, 0) / xs.length))]),
  );
}

export function tiers(scored: Scored[], wl: Workload): Tiers {
  const notes: string[] = [];
  if (scored.length === 0) return { notes: ["no scored executable targets"] };
  const conservative = (s: Scored) => s.mean - s.uncertainty;
  const byQuality = [...scored].sort(
    (a, b) => conservative(b) - conservative(a) || a.target.id.localeCompare(b.target.id),
  );
  const flagship = byQuality[0];
  const next = byQuality[1];
  const strongTie =
    next !== undefined && conservative(flagship) - conservative(next) < wl.tie_margin;
  if (strongTie) notes.push(`flagship and ${next.target.preset} are statistically tied`);
  const cheapest = (pool: Scored[]) =>
    [...pool]
      .filter((s) => s.cost !== null)
      .sort((a, b) => (a.cost as number) - (b.cost as number) || b.mean - a.mean)[0];
  const workhorse = cheapest(
    scored.filter((s) => s.mean >= flagship.mean - wl.noninferiority_margin),
  );
  const value = cheapest(scored.filter((s) => s.mean >= wl.value_floor));
  if (!workhorse) notes.push("no cost evidence for non-inferior targets; workhorse undetermined");
  if (!value) notes.push("no target with cost evidence clears the value floor");
  return {
    flagship: flagship.target.preset,
    strong: next?.target.preset,
    strong_tie: strongTie,
    workhorse: workhorse?.target.preset,
    value: value?.target.preset,
    notes,
  };
}

export function recommendation(t: Tiers): { order: string[]; default?: string } {
  const order = [t.flagship, t.strong, t.workhorse, t.value].filter((x): x is string => !!x);
  return { order: [...new Set(order)], default: t.workhorse };
}

export function readObservations(dirs: string[]): Observation[] {
  const out: Observation[] = [];
  for (const dir of dirs.filter(existsSync)) {
    for (const f of readdirSync(dir)
      .filter((f) => f.endsWith(".jsonl"))
      .sort()) {
      for (const line of readFileSync(join(dir, f), "utf8").split("\n")) {
        if (line.trim()) out.push(JSON.parse(line) as Observation);
      }
    }
  }
  const ids = new Set<string>();
  for (const o of out) {
    if (ids.has(o.id)) throw new Error(`duplicate observation id ${o.id}`);
    ids.add(o.id);
  }
  return out;
}

export function buildCatalog(m: Methodology, targets: Target[], obs: Observation[], asOf: string) {
  const known = new Set(targets.map((t) => t.id));
  const unmapped = [
    ...new Set(obs.filter((o) => !known.has(o.target)).map((o) => o.target)),
  ].sort();
  const { values, dropped } = normalize(obs, m);
  const costs = costIndex(obs);
  const taskOf = new Map(
    Object.entries(m.tasks).flatMap(([t, d]) => d.benchmarks.map((b) => [b, t] as const)),
  );
  const rows = targets.map((t) => {
    const mine = obs.filter((o) => o.target === t.id);
    const tasks: Record<string, Cell> = {};
    for (const task of Object.keys(m.tasks)) {
      tasks[task] = scoreCell(
        mine.filter((o) => taskOf.get(o.benchmark) === task),
        values,
        m,
        asOf,
      );
    }
    const workloads = Object.fromEntries(
      Object.entries(m.workloads).map(([k, wl]) => [k, workloadScore(tasks, wl)]),
    );
    const provisional =
      t.released !== undefined && days(t.released, asOf) < m.freshness.provisional_days;
    return {
      ...t,
      executable: !!(t.provider && t.preset),
      provisional,
      tasks,
      workloads,
      cost_index: costs.get(t.id) ?? null,
    };
  });
  const providers = [
    ...new Set(rows.filter((r) => r.executable).map((r) => r.provider as string)),
  ].sort();
  const tierTable: Record<string, Record<string, Tiers>> = {};
  for (const p of providers) {
    tierTable[p] = {};
    for (const [k, wl] of Object.entries(m.workloads)) {
      const mine = rows.filter((r) => r.provider === p && r.executable);
      // A provisional model may be tiered only on non-vendor evidence for
      // every task the workload scored; launch-week vendor claims never count.
      const admissible = (r: (typeof rows)[number]) =>
        !r.provisional ||
        Object.keys(wl.weights).every(
          (task) =>
            r.tasks[task].status !== "scored" ||
            (r.tasks[task].independent_lineages ?? 0) >= m.evidence.min_lineages,
        );
      const pool = mine.filter((r) => r.workloads[k].status === "scored" && admissible(r));
      const unevaluated = mine.filter((r) => !pool.includes(r)).map((r) => r.preset);
      tierTable[p][k] = tiers(
        pool.map((r) => ({
          target: r,
          mean: r.workloads[k].mean as number,
          uncertainty: r.workloads[k].uncertainty as number,
          cost: r.cost_index,
        })),
        wl,
      );
      if (unevaluated.length)
        tierTable[p][k].notes.push(`not evaluated: ${unevaluated.join(", ")}`);
    }
  }
  const recommendations = Object.fromEntries(
    providers.map((p) => [p, recommendation(tierTable[p].session_default)]),
  );
  return {
    schema: 1,
    as_of: asOf,
    observations: obs.length,
    dropped_groups: dropped,
    unmapped_targets: unmapped,
    targets: rows,
    tiers: tierTable,
    recommendations,
  };
}

export type Catalog = ReturnType<typeof buildCatalog>;

export function summary(c: Catalog): string {
  const lines = [
    `# Model routing summary (${c.as_of})`,
    "",
    "Generated by score.ts from models/; do not edit.",
    "",
  ];
  for (const [p, byWl] of Object.entries(c.tiers)) {
    lines.push(`## ${p}`);
    for (const [wl, t] of Object.entries(byWl)) {
      const parts = (["flagship", "strong", "workhorse", "value"] as const).map(
        (k) => `${k} ${t[k] ?? "?"}`,
      );
      lines.push(
        `- ${wl}: ${parts.join(" · ")}${t.notes.length ? ` (${t.notes.join("; ")})` : ""}`,
      );
    }
    lines.push("");
  }
  const provisional = c.targets.filter((t) => t.provisional).map((t) => t.id);
  if (provisional.length)
    lines.push(`Provisional (tiered only on non-vendor evidence): ${provisional.join(", ")}`);
  const thin = c.targets
    .filter((t) => t.executable && Object.values(t.workloads).every((w) => w.status !== "scored"))
    .map((t) => t.id);
  if (thin.length) lines.push(`Insufficient evidence: ${thin.join(", ")}`);
  return `${lines.join("\n").trimEnd()}\n`;
}

type ProviderPreset = { id: string; is_default?: boolean; values: Record<string, string> };
export function audit(
  c: Catalog,
  manifests: Record<string, { configuration_presets?: ProviderPreset[] }>,
  asOf: string,
) {
  const findings: string[] = [];
  if (days(c.as_of, asOf) > 30)
    findings.push(`catalog is ${Math.round(days(c.as_of, asOf))} days old`);
  for (const [p, manifest] of Object.entries(manifests)) {
    const presets = manifest.configuration_presets ?? [];
    const targets = c.targets.filter((t) => t.provider === p);
    for (const pr of presets) {
      const t = targets.find((t) => t.preset === pr.id);
      if (!t) findings.push(`${p}/${pr.id}: signed preset has no evaluation target`);
      else if (JSON.stringify(sortKeys(t.values ?? {})) !== JSON.stringify(sortKeys(pr.values)))
        findings.push(`${p}/${pr.id}: target values differ from the signed preset`);
    }
    for (const t of targets) {
      if (!presets.some((pr) => pr.id === t.preset))
        findings.push(`${p}/${t.preset}: target has no signed preset`);
    }
    const rec = c.recommendations[p];
    if (!rec) continue;
    const current = presets.map((pr) => pr.id).filter((id) => rec.order.includes(id));
    if (JSON.stringify(current) !== JSON.stringify(rec.order))
      findings.push(`${p}: recommended order ${rec.order.join(" > ")} differs from signed order`);
    const signedDefault = presets.find((pr) => pr.is_default)?.id;
    if (rec.default && rec.default !== signedDefault)
      findings.push(
        `${p}: evidence default ${rec.default} differs from signed default ${signedDefault ?? "none"}`,
      );
  }
  return findings;
}

const sortKeys = (o: Record<string, string>) =>
  Object.fromEntries(Object.entries(o).sort(([a], [b]) => a.localeCompare(b)));

export function digest(text: string): string {
  return `sha256:${createHash("sha256").update(text).digest("hex")}`;
}

function main(argv: string[]) {
  const arg = (name: string) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const root = arg("--models") ?? "models";
  const asOf = arg("--as-of") ?? new Date().toISOString().slice(0, 10);
  const state = process.env.XDG_STATE_HOME ?? join(process.env.HOME ?? "", ".local/state");
  const local = arg("--local-observations") ?? join(state, "cowboy/model-intel/observations");
  const m = Bun.TOML.parse(
    readFileSync(join(root, "methodology.toml"), "utf8"),
  ) as unknown as Methodology;
  const targets = (
    Bun.TOML.parse(readFileSync(join(root, "targets.toml"), "utf8")) as { targets: Target[] }
  ).targets;
  if (argv.includes("--audit")) {
    // Audit the committed catalog: its age and recommendations are what
    // agents and the release workflow actually consume.
    const committed = join(root, "catalog.json");
    if (!existsSync(committed)) {
      console.log(
        JSON.stringify({ as_of: asOf, findings: ["no committed models/catalog.json"] }, null, 2),
      );
      if (argv.includes("--strict")) process.exit(1);
      return;
    }
    const catalog = JSON.parse(readFileSync(committed, "utf8")) as Catalog;
    const manifests = Object.fromEntries(
      [...new Set(targets.map((t) => t.provider).filter((p): p is string => !!p))].map((p) => [
        p,
        JSON.parse(readFileSync(join("plugins", p, "provider.json"), "utf8")),
      ]),
    );
    const findings = audit(catalog, manifests, asOf);
    console.log(JSON.stringify({ as_of: asOf, findings }, null, 2));
    if (argv.includes("--strict") && findings.length) process.exit(1);
    return;
  }
  const obs = readObservations([join(root, "observations"), local]);
  const catalog = buildCatalog(m, targets, obs, asOf);
  const text = `${JSON.stringify(catalog, null, 2)}\n`;
  writeFileSync(join(root, "catalog.json"), text);
  writeFileSync(join(root, "summary.md"), summary(catalog));
  console.log(
    JSON.stringify({
      catalog: digest(text),
      observations: obs.length,
      dropped: catalog.dropped_groups.length,
      unmapped: catalog.unmapped_targets,
    }),
  );
}

if (import.meta.main) main(process.argv.slice(2));
