# Evidence methodology

`models/methodology.toml` holds the numbers; this file explains how to produce
observations that the scorer can treat as comparable.

## Observation record

One JSON object per line in `models/observations/<yyyy-mm>.jsonl`:

```json
{"id":"swe-bench-pro/scale/2026-11-10/gpt-6.2-astra:max",
 "target":"gpt-6.2-astra:max",
 "benchmark":"swe-bench-pro","benchmark_version":"public-2026-09",
 "harness":"scale-seal-standard","axis":"capability",
 "metric":"resolved_rate","unit":"fraction","ceiling":1,"higher_is_better":true,
 "value":0.512,"n":731,"measured_at":"2026-11-10",
 "lineage":"scale-seal/swe-bench-pro/2026-11-10","provenance":"independent",
 "source":{"url":"https://...","publisher":"Scale AI","rights":"redistributable",
           "retrieved_at":"2026-11-18"}}
```

- `target` is a `models/targets.toml` id. A source row whose model alias or
  effort cannot be mapped exactly gets its own research target; never attach
  it to the nearest executable preset. `default` effort is its own target.
- `benchmark` must appear in exactly one `[tasks.*]` list, or it is ignored.
- `lineage` identifies the underlying run. The publisher's page, its API, its
  mirror and a news article about it share one lineage.
- `provenance`: `independent` (publisher ran it), `third_party` (reproduction),
  `vendor` (self-reported), `internal` (Cowboy pilot).
- `axis`: `capability` and `cost` feed scores; `preference` (arena Elo) and
  `adoption` (usage share) are recorded for the report only.
- Cost observations use `axis = "cost"` and a metric that says what is
  measured: `usd_per_task` (API-equivalent), `tokens_per_task`, or
  `minutes_per_task`. Each workload names its `cost_metric`; targets are
  compared only on cost groups (same benchmark, version, harness, metric and
  unit) they share, so record paired measurements whenever possible.
- `ceiling` is required for bounded metrics (fractions, percentages). Unbounded
  metrics (Elo, index points) need at least three targets in one group.

## Admission rules

- Use the primary publisher. Aggregators may only point you at it.
- Do not mix harnesses inside one group: give each harness its own `harness`.
- Record vendor numbers only when no independent run exists, and say so in
  the report.
- A model released within `provisional_days` is recorded but excluded from
  tiers; re-check it on the next refresh.
- Alias retargeting (an alias such as `opus` moving to a new snapshot), a
  harness change, a price change or a withdrawn preset invalidates affected
  observations immediately: set `validity = "superseded"` (and
  `superseded_by` when a replacement exists) instead of deleting them. Only
  `valid` rows (the default) are scored.

## Internal review pilots

Pilots answer the question the public benchmarks do not: does a cheaper preset
find the same serious defects in this fleet's real reviews?

1. Select historical reviews whose worktree still matches the reviewed input
   (clean, no commit after the review started). Re-run them with the candidate
   presets using the original focus text, `--root`, `--base` and a new `--task`.
2. Use the union of serious findings (high/critical, or medium with concrete
   data-loss/security/contract impact) across all arms as the reference set.
   Adjudicate blind: give an adjudicator from a different model family the
   arms anonymised (A, B, …), have it cluster findings by root cause and
   verify each cluster read-only against the reviewed code, and keep the
   arm key out of its prompt. Commit the cluster summary under
   `models/pilots/`.
3. Per arm record recall of the verified serious set, unsupported findings
   (false positives), tokens and minutes. Record recall as a capability
   observation (`ceiling = 1`), tokens/minutes as cost observations, one
   lineage per pilot input, and the input id as `benchmark_version` so each
   input is its own paired group. Until step 2's verification is complete,
   mark the rows `validity = "unverified"`.
4. A pilot with fewer than about ten inputs is directional only: say so, and
   do not let it alone move a default.

## Normalization and tiers

The scorer first keeps one row per (target, group, lineage) — most trusted,
then newest — so mirrors never set a scale or count twice. It normalizes
bounded metrics as the oriented fraction of their ceiling and unbounded ones
by min-max within the group (three targets minimum), drops saturated or flat
groups, weights lineages by provenance and a freshness half-life, and refuses
to score a cell below `min_lineages`. Candidates for a tier are compared only
on the weighted tasks they all scored, which must cover
`min_workload_coverage`. Tier rules are documented in `methodology.toml`.
