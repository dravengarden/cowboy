---
name: model-intel
description: Compile objective, provenance-tracked evidence on which model and reasoning effort fits each task type at what cost (Claude, GPT/Codex, Gemini, Grok, DeepSeek and others), derive flagship/strong/workhorse/value tiers per Cowboy Provider, and propose Recommended preset and review-selection changes. Use when refreshing the model catalog, when a Provider upgrade adds or retires models, or when asked which model or effort a task should use.
---

# Model intel

Keep this skill canonical in the Cowboy repository. It owns the evidence in
`models/`; executable presets stay in signed `plugins/<id>/provider.json` and
are changed only through `release-cowboy-plugin`.

## Outputs

| Path | Owner | Purpose |
| --- | --- | --- |
| `models/methodology.toml` | reviewed by hand | task taxonomy, weights, floors, tier rules |
| `models/targets.toml` | reviewed by hand | (model, effort) configurations; executable ones bind a signed preset |
| `models/observations/*.jsonl` | this workflow | redistributable measurements, one per line |
| `models/catalog.json` | `scripts/score.ts` | per-target task cells, workload scores, tiers, recommendations |
| `models/summary.md` | `scripts/score.ts` | short routing summary for agents; never hand-edited |
| `models/report.md` | this workflow | detailed report: sources, disagreements, gaps, decisions |

Reference-only measurements (sources whose terms forbid redistribution) stay
in `$XDG_STATE_HOME/cowboy/model-intel/observations/`. The scorer rejects them
in `models/observations/`; `catalog.json` records the count and digest of
both input sets so a later audit can tell whether its inputs are complete. Read
[sources](references/sources.md) for the source registry and rights.

## Workflow

1. **Decide the scope.** Run `scripts/score.ts --audit` first (from the
   repository root, in the dev shell). A full refresh is warranted when it
   reports drift, the catalog is older than 30 days, a Provider release adds,
   retires or retargets a model, or a source publishes a material new run.
   Otherwise answer from `models/summary.md`.
2. **Inventory executable targets.** Read `cowboy call capabilities` and the
   signed `configuration_presets`. Every executable target in
   `models/targets.toml` must carry the exact preset `values`; the audit
   rejects mismatches. Add a market-research-only target (no `provider`) for a
   model Cowboy cannot call; it is reported but never recommended.
3. **Collect observations** following [methodology](references/methodology.md):
   primary sources only, one observation per (target, benchmark, harness,
   metric, date), with lineage, provenance, sample size and rights. Aggregator
   sites are discovery aids, never observations.
4. **Add internal evidence.** Paired Cowboy pilots on identical review inputs
   are the most relevant evidence for `review` and `design_review`. Record them
   as `provenance = "internal"`, benchmark `cowboy-review-pilot`, with
   `validity = "unverified"` until every reference finding has been checked
   against the code; unverified rows are reported but never scored.
5. **Score.** `nix develop -c bun .agents/skills/model-intel/scripts/score.ts
   --as-of YYYY-MM-DD`. It refuses to run without the local reference-only
   inputs; `--repository-only` is for a deliberate partial catalog. Inspect
   `dropped_groups`, `unmapped_targets`, insufficient cells and provisional
   targets before reading any tier.
6. **Write `models/report.md`.** Lead with the decisions the evidence supports,
   then per-Provider tiers, disagreements between sources, provisional models,
   evidence gaps and every source with its retrieval date. State what the
   evidence cannot decide; never fill a gap with a vendor claim or intuition.
7. **Review.** A methodology change (weights, floors, task mapping) needs one
   design review before its catalog is committed. A data-only refresh needs a
   normal review of the catalog and report diff.
8. **Deliver.** Commit `models/`. If the recommendations differ from the
   signed presets, apply the proposed order/default through
   `release-cowboy-plugin` (preserving preset IDs; fewer than four distinct
   recommendations is acceptable) in a separate, reviewable change.

## Using the catalog to pick a model

For a single call, prefer the Provider's tier for the matching workload in
`models/summary.md` when one is determined: `workhorse` for ordinary work, `flagship` only for high
risk (security, permissions, concurrency, persistence, deletion, money, large
refactors) or after a workhorse result is inconclusive. An explicit user
choice, session preference or Settings target always wins. An undetermined
tier (`?`) means the evidence cannot decide; keep the existing selection
rather than guessing, and never treat a provisional target as recommended.

## Guardrails

- Keep capability, preference (arena votes), adoption (usage share) and vendor
  claims as separate axes; only capability and cost feed tiers.
- Count a measurement once per lineage, however many sites republish it.
- API list price times tokens is API-equivalent cost, not subscription quota;
  label which one each cost observation measures.
- A catalog age or a missing model never blocks an unrelated Provider fix.
