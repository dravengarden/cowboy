// Deterministic model evidence scorer.
//
// Reads models/methodology.toml, models/targets.toml and observation JSONL
// files, then writes models/catalog.json and models/summary.md. `--audit`
// checks the committed catalog and current targets against the signed
// Provider presets without writing. Judgment (which sources to trust, what a
// disagreement means) belongs to the skill; every number here is a pure
// function of the versioned inputs.

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export type Provenance = "independent" | "third_party" | "vendor" | "internal";
export type Rights = "redistributable" | "reference_only";
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
  // Only `valid` (the default) feeds scores. `unverified` (for example an
  // unadjudicated pilot) and `superseded` rows stay as history.
  validity?: "valid" | "unverified" | "superseded";
  superseded_by?: string;
  source: { url: string; publisher: string; rights: Rights; retrieved_at?: string };
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
export type Workload = {
  weights: Record<string, number>;
  cost_metric: string;
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
    min_workload_coverage: number;
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
  max_age_days?: number;
  confidence?: "high" | "medium" | "low";
};
export type Candidate = {
  preset: string;
  cells: Record<string, Cell>;
  costs: Map<string, number>; // cost group -> value for the workload's metric
};
export type Tiers = {
  flagship?: string;
  tied_with_flagship: string[];
  strong?: string;
  workhorse?: string;
  value?: string;
  compared_tasks: string[];
  notes: string[];
};

const DAY_MS = 86_400_000;
const round = (x: number) => Math.round(x * 1000) / 1000;
const days = (from: string, to: string) => (Date.parse(to) - Date.parse(from)) / DAY_MS;

// Rows measured under one protocol share a group; unit, bound and direction
// are part of the identity so incompatible scales never mix.
export function groupKey(o: Observation): string {
  return [
    o.benchmark,
    o.benchmark_version ?? "",
    o.harness ?? "",
    o.metric,
    o.unit,
    o.ceiling ?? "",
    o.higher_is_better === false ? "lower" : "higher",
  ].join("|");
}

// One row per (target, group, lineage): the most trusted, then the newest.
// Mirrors of a run therefore never influence extrema or weights twice.
export function canonical(obs: Observation[], m: Methodology): Observation[] {
  const best = new Map<string, Observation>();
  for (const o of obs) {
    if ((o.validity ?? "valid") !== "valid") continue;
    const k = `${o.target}#${groupKey(o)}#${o.lineage}`;
    const prev = best.get(k);
    const better =
      !prev ||
      m.provenance[o.provenance] > m.provenance[prev.provenance] ||
      (m.provenance[o.provenance] === m.provenance[prev.provenance] &&
        o.measured_at > prev.measured_at);
    if (better) best.set(k, o);
  }
  return [...best.values()].sort((a, b) => a.id.localeCompare(b.id));
}

function groupBy<T>(xs: T[], key: (x: T) => string): Map<string, T[]> {
  const out = new Map<string, T[]>();
  for (const x of xs) out.set(key(x), [...(out.get(key(x)) ?? []), x]);
  return new Map([...out].sort(([a], [b]) => a.localeCompare(b)));
}

// Normalize canonical capability rows to [0, 1], oriented so higher is
// better. Bounded metrics use the fraction of their ceiling; unbounded ones
// use min-max within the group and need three targets. Saturated or flat
// groups are dropped and reported.
export function normalize(
  rows: Observation[],
  m: Methodology,
): { values: Map<string, number>; dropped: string[] } {
  const values = new Map<string, number>();
  const dropped: string[] = [];
  for (const [k, group] of groupBy(
    rows.filter((o) => o.axis === "capability"),
    groupKey,
  )) {
    const lower = group[0].higher_is_better === false;
    const targets = new Set(group.map((o) => o.target)).size;
    const ceiling = group[0].ceiling;
    if (ceiling !== undefined) {
      const oriented = group.map((o) => (lower ? 1 - o.value / ceiling : o.value / ceiling));
      const hi = Math.max(...oriented);
      const lo = Math.min(...oriented);
      // Saturated means most targets sit near the ceiling, not that one does:
      // a discriminating benchmark may well have a perfect leader.
      const sorted = [...oriented].sort((a, b) => a - b);
      const median = sorted[Math.floor((sorted.length - 1) / 2)];
      if (
        median > m.evidence.saturation_fraction ||
        (targets > 1 && hi - lo < m.evidence.min_spread)
      ) {
        dropped.push(`${k}: saturated or flat`);
        continue;
      }
      group.forEach((o, i) => values.set(o.id, oriented[i]));
      continue;
    }
    const signed = group.map((o) => (lower ? -o.value : o.value));
    const hi = Math.max(...signed);
    const lo = Math.min(...signed);
    if (targets < 3 || hi === lo) {
      dropped.push(`${k}: fewer than 3 targets for relative scale`);
      continue;
    }
    group.forEach((o, i) => values.set(o.id, (signed[i] - lo) / (hi - lo)));
  }
  return { values, dropped };
}

// A lineage contributes one value per task: the mean of its normalized rows.
export function scoreCell(
  rows: Observation[],
  values: Map<string, number>,
  m: Methodology,
  asOf: string,
): Cell {
  const lineages = [
    ...groupBy(
      rows.filter((o) => values.has(o.id)),
      (o) => o.lineage,
    ).values(),
  ].map((ls) => {
    const value = ls.reduce((s, o) => s + (values.get(o.id) as number), 0) / ls.length;
    const age = Math.max(0, Math.min(...ls.map((o) => days(o.measured_at, asOf))));
    const trust = Math.max(...ls.map((o) => m.provenance[o.provenance]));
    return { value, age, weight: trust * 0.5 ** (age / m.freshness.half_life_days) };
  });
  if (lineages.length < m.evidence.min_lineages) {
    return { status: "insufficient", lineages: lineages.length };
  }
  const w = lineages.reduce((s, l) => s + l.weight, 0);
  const mean = lineages.reduce((s, l) => s + l.value * l.weight, 0) / w;
  const variance = lineages.reduce((s, l) => s + l.weight * (l.value - mean) ** 2, 0) / w;
  const uncertainty = Math.max(m.evidence.uncertainty_floor, Math.sqrt(variance / lineages.length));
  const newest = Math.min(...lineages.map((l) => l.age));
  const confidence =
    lineages.length >= m.evidence.high_confidence_lineages &&
    newest <= m.evidence.high_confidence_max_age_days
      ? "high"
      : "medium";
  return {
    status: "scored",
    mean: round(mean),
    uncertainty: round(uncertainty),
    lineages: lineages.length,
    max_age_days: Math.round(newest),
    confidence,
  };
}

// Score every candidate on the same task set: the weighted tasks all of them
// scored. Different task mixes are never ranked against each other.
export function commonScores(candidates: Candidate[], wl: Workload, minCoverage: number) {
  const tasks = Object.keys(wl.weights)
    .filter((t) => wl.weights[t] > 0)
    .filter((t) => candidates.every((c) => c.cells[t]?.status === "scored"))
    .sort();
  const covered = tasks.reduce((s, t) => s + wl.weights[t], 0);
  if (candidates.length === 0 || covered < minCoverage) return { tasks, scores: null };
  const scores = candidates.map((c) => {
    const mean = tasks.reduce((s, t) => s + wl.weights[t] * (c.cells[t].mean as number), 0);
    const variance = tasks.reduce(
      (s, t) => s + wl.weights[t] ** 2 * (c.cells[t].uncertainty as number) ** 2,
      0,
    );
    return { c, mean: mean / covered, uncertainty: Math.sqrt(variance) / covered };
  });
  return { tasks, scores };
}

// Geometric mean of a/b over the cost groups both were measured in, or null.
export function costRatio(a: Candidate, b: Candidate): number | null {
  const shared = [...a.costs.keys()].filter((g) => b.costs.has(g));
  if (shared.length === 0) return null;
  const log = shared.reduce(
    (s, g) => s + Math.log((a.costs.get(g) as number) / (b.costs.get(g) as number)),
    0,
  );
  return Math.exp(log / shared.length);
}

type Ranked = { c: Candidate; mean: number; uncertainty: number };
// The cheapest candidate must be directly comparable with every other one.
function cheapest(pool: Ranked[]): Ranked | undefined {
  const winners = pool.filter(
    (x) =>
      x.c.costs.size > 0 &&
      pool.every((y) => {
        if (y === x) return true;
        const r = costRatio(x.c, y.c);
        return r !== null && r <= 1;
      }),
  );
  return winners.sort((a, b) => b.mean - a.mean || a.c.preset.localeCompare(b.c.preset))[0];
}

export function tiers(candidates: Candidate[], wl: Workload, minCoverage: number): Tiers {
  const { tasks, scores } = commonScores(candidates, wl, minCoverage);
  const base: Tiers = { tied_with_flagship: [], compared_tasks: tasks, notes: [] };
  if (candidates.length === 0) return { ...base, notes: ["no admissible executable targets"] };
  if (!scores) {
    return { ...base, notes: ["candidates do not share enough scored tasks to be ranked"] };
  }
  // An uncontested candidate is not evidence of being the best choice.
  if (scores.length < 2) {
    return {
      ...base,
      notes: [`only ${scores[0].c.preset} is rankable; tiers need two candidates`],
    };
  }
  const conservative = (s: Ranked) => s.mean - s.uncertainty;
  const ranked = [...scores].sort(
    (a, b) => conservative(b) - conservative(a) || a.c.preset.localeCompare(b.c.preset),
  );
  const flagship = ranked[0];
  const gap = (s: Ranked) => conservative(flagship) - conservative(s);
  const tied = ranked.slice(1).filter((s) => gap(s) < wl.tie_margin);
  const strong = ranked.slice(1).find((s) => gap(s) >= wl.tie_margin);
  const workhorse = cheapest(
    scores.filter((s) => s.mean >= flagship.mean - wl.noninferiority_margin),
  );
  const value = cheapest(scores.filter((s) => s.mean >= wl.value_floor));
  const notes: string[] = [];
  if (tied.length)
    notes.push(
      `within the configured tie margin of flagship: ${tied.map((s) => s.c.preset).join(", ")}`,
    );
  if (!workhorse) notes.push(`no comparable ${wl.cost_metric} evidence among non-inferior targets`);
  if (!value)
    notes.push(`no comparable ${wl.cost_metric} evidence among targets above the value floor`);
  return {
    ...base,
    flagship: flagship.c.preset,
    tied_with_flagship: tied.map((s) => s.c.preset),
    strong: strong?.c.preset,
    workhorse: workhorse?.c.preset,
    value: value?.c.preset,
    notes,
  };
}

export function recommendation(t: Tiers): { order: string[]; default?: string } {
  const order = [t.flagship, t.strong, t.workhorse, t.value].filter((x): x is string => !!x);
  return { order: [...new Set(order)], default: t.workhorse };
}

export type Inputs = { rows: Observation[]; count: number; digest: string; present: boolean };

// Repository inputs must be redistributable; the local directory may hold
// reference-only rows. The digest covers the exact bytes read.
export function readObservations(dir: string, allowRestricted: boolean): Inputs {
  if (!existsSync(dir)) return { rows: [], count: 0, digest: digest(""), present: false };
  const hash = createHash("sha256");
  const rows: Observation[] = [];
  for (const f of readdirSync(dir)
    .filter((f) => f.endsWith(".jsonl"))
    .sort()) {
    const text = readFileSync(join(dir, f), "utf8");
    hash.update(`${f}\n${text}`);
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      const o = JSON.parse(line) as Observation;
      if (!allowRestricted && o.source?.rights !== "redistributable") {
        throw new Error(`${dir}/${f}: ${o.id} is not redistributable; keep it in local state`);
      }
      rows.push(o);
    }
  }
  return { rows, count: rows.length, digest: `sha256:${hash.digest("hex")}`, present: true };
}

const binding = (t: Target) => ({
  id: t.id,
  provider: t.provider,
  preset: t.preset,
  values: sortKeys(t.values ?? {}),
});

export type ConfigDigests = { methodology: string; targets: string };

export function buildCatalog(
  m: Methodology,
  targets: Target[],
  repo: Inputs,
  local: Inputs,
  asOf: string,
  config: ConfigDigests = { methodology: "", targets: "" },
) {
  const all = [...repo.rows, ...local.rows];
  const ids = new Set<string>();
  for (const o of all) {
    if (ids.has(o.id)) throw new Error(`duplicate observation id ${o.id}`);
    ids.add(o.id);
  }
  const known = new Set(targets.map((t) => t.id));
  const unmapped = [
    ...new Set(all.filter((o) => !known.has(o.target)).map((o) => o.target)),
  ].sort();
  // Unmapped targets and evidence measured after the snapshot never shape
  // scales or weights for mapped targets.
  const future = all.filter((o) => o.measured_at > asOf).length;
  const rows = canonical(
    all.filter((o) => known.has(o.target) && o.measured_at <= asOf),
    m,
  );
  const { values, dropped } = normalize(rows, m);
  const taskOf = new Map(
    Object.entries(m.tasks).flatMap(([t, d]) => d.benchmarks.map((b) => [b, t] as const)),
  );
  const scored = targets.map((t) => {
    const mine = rows.filter((o) => o.target === t.id);
    const tasks = Object.fromEntries(
      Object.keys(m.tasks).map((task) => [
        task,
        scoreCell(
          mine.filter((o) => taskOf.get(o.benchmark) === task),
          values,
          m,
          asOf,
        ),
      ]),
    );
    const costGroups = groupBy(
      mine.filter((o) => o.axis === "cost" && o.value > 0),
      (o) => o.metric,
    );
    const cost = Object.fromEntries(
      [...costGroups].map(([metric, os]) => [metric, new Set(os.map(groupKey)).size]),
    );
    const provisional =
      t.released !== undefined && days(t.released, asOf) < m.freshness.provisional_days;
    return { ...t, executable: !!(t.provider && t.preset), provisional, tasks, cost_groups: cost };
  });
  const costOf = (t: Target, metric: string) => {
    const mine = rows.filter(
      (o) => o.target === t.id && o.axis === "cost" && o.metric === metric && o.value > 0,
    );
    // Several lineages in one group (for example repeated runs) average.
    return new Map(
      [...groupBy(mine, groupKey)].map(([g, os]) => [
        g,
        os.reduce((s, o) => s + o.value, 0) / os.length,
      ]),
    );
  };
  const providers = [
    ...new Set(scored.filter((r) => r.executable).map((r) => r.provider as string)),
  ].sort();
  const tierTable: Record<string, Record<string, Tiers>> = {};
  for (const p of providers) {
    tierTable[p] = {};
    for (const [k, wl] of Object.entries(m.workloads)) {
      const mine = scored.filter((r) => r.provider === p && r.executable);
      const pool = mine.filter((r) => !r.provisional);
      const candidates = pool.map((r) => ({
        preset: r.preset as string,
        cells: r.tasks,
        costs: costOf(r, wl.cost_metric),
      }));
      const t = tiers(candidates, wl, m.evidence.min_workload_coverage);
      const provisional = mine.filter((r) => r.provisional).map((r) => r.preset);
      if (provisional.length) t.notes.push(`provisional, not ranked: ${provisional.join(", ")}`);
      tierTable[p][k] = t;
    }
  }
  const recommendations = Object.fromEntries(
    providers.map((p) => [p, recommendation(tierTable[p].session_default)]),
  );
  return {
    schema: 2,
    as_of: asOf,
    inputs: {
      repository: { count: repo.count, digest: repo.digest },
      local: { count: local.count, digest: local.digest },
      canonical_rows: rows.length,
      normalized_capability_rows: values.size,
      excluded_after_snapshot: future,
      ...config,
    },
    bindings: targets.filter((t) => t.provider && t.preset).map(binding),
    dropped_groups: dropped,
    unmapped_targets: unmapped,
    targets: scored,
    tiers: tierTable,
    recommendations,
  };
}

export type Catalog = ReturnType<typeof buildCatalog>;

// Served verbatim by Cowboy MCP help("models"): keep it small and decisive.
// Only determined tiers are listed; per-preset evidence follows so a caller
// can still compare presets when no tier is determined.
export function summary(c: Catalog): string {
  const lines = [
    `# Model routing summary (${c.as_of})`,
    "Generated by score.ts; do not edit. `?`/absent tier = evidence cannot decide: keep the current selection.",
    "",
    "## Tiers",
  ];
  const tierKeys = ["flagship", "strong", "workhorse", "value"] as const;
  let anyTier = false;
  for (const [p, byWl] of Object.entries(c.tiers)) {
    for (const [wl, t] of Object.entries(byWl)) {
      if (!tierKeys.some((k) => t[k])) continue;
      anyTier = true;
      lines.push(`- ${p}/${wl}: ${tierKeys.map((k) => `${k} ${t[k] ?? "?"}`).join(" · ")}`);
    }
  }
  if (!anyTier) lines.push("- none determined yet");
  lines.push("", "## Evidence per preset (mean±uncertainty/lineages; normalized 0-1)");
  for (const t of c.targets) {
    if (!t.executable) continue;
    const cells = Object.entries(t.tasks)
      .filter(([, cell]) => cell.status === "scored")
      .map(
        ([task, cell]) =>
          `${task} ${cell.mean?.toFixed(2)}±${cell.uncertainty?.toFixed(2)}/${cell.lineages}`,
      );
    if (cells.length === 0) continue;
    lines.push(
      `- ${t.provider}/${t.preset}${t.provisional ? " (provisional)" : ""}: ${cells.join(" · ")}`,
    );
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

type ProviderPreset = { id: string; is_default?: boolean; values: Record<string, string> };
type Manifest = { configuration_presets?: ProviderPreset[] };

export function audit(
  c: Catalog,
  targets: Target[],
  manifests: Record<string, Manifest>,
  current: { repository: string; local: string | null } & Partial<ConfigDigests>,
  asOf: string,
  provisionalDays = 14,
): string[] {
  const findings: string[] = [];
  const age = Math.round(days(c.as_of, asOf));
  if (age > 30) findings.push(`catalog is ${age} days old`);
  for (const key of ["methodology", "targets"] as const) {
    if (current[key] !== undefined && current[key] !== c.inputs[key]) {
      findings.push(`${key}.toml changed since the catalog was generated`);
    }
  }
  const matured = c.targets
    .filter((t) => t.provisional && t.released && days(t.released, asOf) >= provisionalDays)
    .map((t) => t.id);
  if (matured.length) findings.push(`provisional window ended for ${matured.join(", ")}; refresh`);
  if (current.repository !== c.inputs.repository.digest) {
    findings.push("repository observations changed since the catalog was generated");
  }
  if (current.local === null)
    findings.push("local reference-only observations unavailable; local inputs not verified");
  else if (current.local !== c.inputs.local.digest)
    findings.push("local observations differ from the catalog's inputs");
  const now = JSON.stringify(targets.filter((t) => t.provider && t.preset).map(binding));
  if (now !== JSON.stringify(c.bindings ?? []))
    findings.push("targets.toml bindings changed since the catalog was generated");
  const providers = [
    ...new Set([
      ...Object.keys(manifests),
      ...targets.flatMap((t) => (t.provider ? [t.provider] : [])),
    ]),
  ].sort();
  for (const p of providers) {
    const presets = manifests[p]?.configuration_presets ?? [];
    const mine = targets.filter((t) => t.provider === p);
    for (const pr of presets) {
      const t = mine.find((t) => t.preset === pr.id);
      if (!t) findings.push(`${p}/${pr.id}: signed preset has no evaluation target`);
      else if (JSON.stringify(sortKeys(t.values ?? {})) !== JSON.stringify(sortKeys(pr.values))) {
        findings.push(`${p}/${pr.id}: target values differ from the signed preset`);
      }
    }
    for (const t of mine) {
      if (!presets.some((pr) => pr.id === t.preset))
        findings.push(`${p}/${t.preset}: target has no signed preset`);
    }
    const rec = c.recommendations[p];
    if (!rec || rec.order.length === 0) continue;
    const leading = presets.slice(0, rec.order.length).map((pr) => pr.id);
    if (JSON.stringify(leading) !== JSON.stringify(rec.order)) {
      findings.push(
        `${p}: evidence order ${rec.order.join(" > ")} differs from signed leading cards ${leading.join(" > ")}`,
      );
    }
    const signedDefault = presets.find((pr) => pr.is_default)?.id;
    if (rec.default && rec.default !== signedDefault) {
      findings.push(
        `${p}: evidence default ${rec.default} differs from signed default ${signedDefault ?? "none"}`,
      );
    }
  }
  return findings;
}

const sortKeys = (o: Record<string, string>) =>
  Object.fromEntries(Object.entries(o).sort(([a], [b]) => a.localeCompare(b)));

export function digest(text: string): string {
  return `sha256:${createHash("sha256").update(text).digest("hex")}`;
}

function agentManifests(): Record<string, Manifest> {
  const out: Record<string, Manifest> = {};
  for (const id of readdirSync("plugins").sort()) {
    const path = join("plugins", id, "provider.json");
    if (!existsSync(path)) continue;
    const manifest = JSON.parse(readFileSync(path, "utf8")) as Manifest;
    if (manifest.configuration_presets?.length) out[id] = manifest;
  }
  return out;
}

function main(argv: string[]) {
  const arg = (name: string) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const root = arg("--models") ?? "models";
  const asOf = arg("--as-of") ?? new Date().toISOString().slice(0, 10);
  const state = process.env.XDG_STATE_HOME ?? join(process.env.HOME ?? "", ".local/state");
  const localDir = arg("--local-observations") ?? join(state, "cowboy/model-intel/observations");
  const methodologyText = readFileSync(join(root, "methodology.toml"), "utf8");
  const targetsText = readFileSync(join(root, "targets.toml"), "utf8");
  const config = { methodology: digest(methodologyText), targets: digest(targetsText) };
  const m = Bun.TOML.parse(methodologyText) as unknown as Methodology;
  const targets = (Bun.TOML.parse(targetsText) as { targets: Target[] }).targets;
  const repositoryOnly = argv.includes("--repository-only");
  const repo = readObservations(join(root, "observations"), false);
  const empty: Inputs = { rows: [], count: 0, digest: digest(""), present: false };
  const local =
    repositoryOnly && !argv.includes("--audit") ? empty : readObservations(localDir, true);
  if (argv.includes("--audit")) {
    const committed = join(root, "catalog.json");
    const findings = existsSync(committed)
      ? audit(
          JSON.parse(readFileSync(committed, "utf8")) as Catalog,
          targets,
          agentManifests(),
          { repository: repo.digest, local: local.present ? local.digest : null, ...config },
          asOf,
          m.freshness.provisional_days,
        )
      : ["no committed models/catalog.json"];
    console.log(JSON.stringify({ as_of: asOf, findings }, null, 2));
    if (argv.includes("--strict") && findings.length) process.exit(1);
    return;
  }
  // Refuse to replace the published catalog from a partial input set.
  const fail = (message: string) => {
    console.error(message);
    process.exit(2);
  };
  if (!repo.present || repo.count === 0)
    fail(`no repository observations under ${root}/observations`);
  if (!repositoryOnly && local.count === 0) {
    fail(
      `no local observations at ${localDir}; pass --repository-only to score repository rows alone`,
    );
  }
  const committed = join(root, "catalog.json");
  if (existsSync(committed) && !argv.includes("--allow-input-reduction")) {
    const previous = (JSON.parse(readFileSync(committed, "utf8")) as Catalog).inputs;
    const reduced =
      repo.count < previous.repository.count ||
      (!repositoryOnly && local.count < previous.local.count);
    if (reduced)
      fail(
        "fewer observations than the committed catalog; pass --allow-input-reduction if intended",
      );
  }
  const catalog = buildCatalog(m, targets, repo, local, asOf, config);
  const text = `${JSON.stringify(catalog, null, 2)}\n`;
  writeFileSync(join(root, "catalog.json"), text);
  writeFileSync(join(root, "summary.md"), summary(catalog));
  console.log(
    JSON.stringify({
      catalog: digest(text),
      inputs: catalog.inputs,
      dropped: catalog.dropped_groups.length,
      unmapped: catalog.unmapped_targets,
    }),
  );
}

if (import.meta.main) main(process.argv.slice(2));
