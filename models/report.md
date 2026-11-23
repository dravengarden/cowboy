# Model evidence report — 2026-11-18

First run of the `model-intel` skill. Scores and tiers are in
`models/catalog.json`; the agent-facing digest is `models/summary.md`.

Inputs: 90 repository observations (`sha256:d1a9d82b…6ef`, including the
verified review pilot) and 164 reference-only observations kept in local state
(`sha256:3cfb64c1…01fc`). After canonical selection 221 valid rows remained,
118 of them normalized capability rows. Numbers from sources whose terms
forbid redistribution are not reproduced here; statements derived from them
are marked *(local inputs)* and can be checked only against that digest.

## Update 2026-11-23: Fable 5.2 maturity, post-cutover field costs and periodic audit

- **Provisional maturity:** `claude-fable-5-2:max` and `claude-fable-5-2:high` (released 2026-11-09) cleared the 14-day provisional window on 2026-11-23; `models/catalog.json` and `models/summary.md` were regenerated with `--as-of 2026-11-23`. Tiers remain undetermined until candidates share >= 70% weighted task coverage (GPT-6.2 exits provisional on 2026-12-01; below-max `agentic_coding` and `code_review` coverage remain single-lineage or absent).
- **Post-cutover field costs (`models/field/2026-11-23.json`, 14-day ledger, reuse excluded):** GPT-6.2 Sol Max grew from n=9 to n=92 (median 3.68M tokens, 9 min) versus GPT-6.2 Astra Max (n=129, median 6.42M tokens, 19 min); Claude Opus High now has initial field cost evidence (n=3, median 2.42M tokens, 5 min). `models/routing.json` field costs were refreshed from this snapshot.
- **Audit loop:** `score.ts` no longer lets a single unmeasured preset veto measured presets of the same Provider and now audits `models/routing.json` and `models/field/` freshness. `scripts/periodic-audit.ts` unifies catalog/routing audit, live ledger telemetry (reviews, analyses and >=30% cost-drift detection), harness inventory and upstream dependency checks (npm, Antigravity CLI GitHub releases, workspace-local drift and git pins).

## Update 2026-11-21: Sol Max arm, field costs and routing

**Sol Max arm.** GPT-6.2 Sol Max re-ran the four pilot inputs (same clean
worktrees, input revisions, base and focus). Matched against the 14
blind-verified serious defects it found 12 (keeper 4/5, pr8818 5/5, pr8895
3/4), using 12.9M tokens and 29 minutes in total against Astra Max's 23.5M and
116. It missed keeper K8 (retirement not gated on recovery feasibility) and
pr8895 Q2 (a later approval push hides an earlier failed push). The match was
done by the orchestrating agent, unblinded; its new findings were not
adjudicated, so the rows are `validity = "unverified"` and are not scored.
Summary: `models/pilots/2026-11-21-sol-max-arm.json`.

**Field costs (Cowboy's own feedback).** `scripts/field.ts` aggregated the
Hawk ledger's completed reviews for 14 days to 2026-11-21
(`models/field/2026-11-21.json`, aggregates only; reused receipts excluded; the Sol Max row includes the four pilot runs):

| Review target | n | Median tokens | Median uncached | Median minutes |
| --- | --- | --- | --- | --- |
| GPT-6 Astra Max | 326 | 1.88M | 150k | 9 |
| GPT-6.2 Astra Max | 101 | 6.68M | 257k | 20 |
| GPT-6.2 Sol Max | 9 | 3.87M | 238k | 9 |
| GPT-6.2 Sol Medium | 5 | 0.80M | 76k | 3 |
| GPT-6.2 Astra Medium | 10 | 0.27M | 43k | 3 |
| Claude Fable 5.1 Max | 3 | 1.29M | 155k | 10 |

The signed `astra-max` preset moved from GPT-6 Astra to GPT-6.2 Astra during
the window (codex 0.169, first GPT-6.2 reviews on 2026-11-18). On the field mix each GPT-6.2 Astra Max review costs
about 3.5× the GPT-6 one in tokens and twice the time: 2026-11-19 used
416M review tokens for 60 reviews, against 256M for 91 on 2026-11-17.
Field rows compare cost only: inputs differ between groups.

**Decision.** `models/routing.json` (docs/review-routing.md) now routes
routine reviews to GPT-6.2 Sol Max and Claude Opus, keeps Astra Max for
declared high risk, and Fable only by explicit high-risk choice. It carries
both pilot and field costs; Claude pilot costs are `unknown`. Next: a blind
adjudication of routine Sol Max reviews after a week of field data, and a
paired Opus/Fable review pilot from a non-Claude parent session.

## What the evidence decides

The catalog determines **no tier for any executable Codex or Claude preset**
yet, and the signed Recommended order and defaults stay unchanged:

- Every GPT-6.2 and Claude Fable 5.2 preset is provisional (released within 14
  days); they are scored for inspection but not ranked until 2026-12-01 and
  2026-11-23 respectively.
- The remaining Claude presets and Luna do not share enough independently
  scored tasks to be ranked against each other (agentic coding below max
  effort has one public lineage each).
- The verified review pilot scores `code_review` for GPT-6.2 Astra Max,
  Astra Medium and Sol Medium, but those presets are provisional; the review
  workload also needs agentic-coding coverage shared by all candidates.

Luna is the only rankable non-provisional Codex target; a single candidate
is never tiered, so no Codex workload has a tier until GPT-6.2 matures.

## What the verified pilot shows

On four real reviews (14 verified serious defects), **GPT-6.2 Astra Max found
all 14**; Astra Medium and Sol Medium each found 9 (64%), and the historical
GPT-6 Astra Max found 8 (57%). Max cost about 11× Astra Medium's tokens and
6× Sol Medium's, and about 6–8× their wall time. An earlier unverified
reading suggested Medium was as good as Max; that came from comparing 6.2
Medium with the older GPT-6 Max and from an unverified reference set, and is
withdrawn.

Implication for routing, pending more inputs: keep Max as the default for
reviews requested for real risk (the built-in `max` routine effort does), and
treat `standard` effort as a deliberate trade — roughly a third of serious
defects missed for an order of magnitude fewer tokens — suitable only for
low-stakes reviews. Four inputs, all chosen because a review was warranted,
are still a small sample.

## Baseline: what managed calls used

Read-only query of the Hawk Controller ledger (`managed_agent_calls`, 934
calls created 2026-10-09 → 2026-11-18 06:23 UTC). Astra Max served 593
reviews, 126 design reviews and 24 analysis calls (about 80% of all calls).
Median Astra Max review: 1.8M tokens (146k uncached input), 7.8 minutes. The
11 Astra Medium reviews had a median of 0.15M tokens and 0.9 minutes, on
unpaired inputs. Cause: the review selector orders the GPT family Astra first
and effort max first, and takes the first candidate when no preset is passed
(`components/server/src/server/managed_calls/selection.rs`).

## Review pilot

Four historical reviews whose worktrees were still clean and unchanged were
re-run with the original focus text and base on GPT-6.2 Astra Max, Astra
Medium and Sol Medium; the original GPT-6 Astra Max result is the fourth
arm. Claude Opus 5.5 adjudicators received the arms anonymised as A–D,
clustered all findings by root cause, and verified each cluster read-only
against the reviewed code (confirmed / refuted / uncertain, and whether it
is serious). Summary: `models/pilots/2026-11-18-review-pilot.json`; rows:
`models/observations/2026-11-cowboy-pilot.jsonl` (the earlier unverified rows
are kept as `superseded`).

| Input | Serious | 6.2 Astra Max | 6.2 Astra Medium | 6.2 Sol Medium | GPT-6 Astra Max |
| --- | --- | --- | --- | --- | --- |
| keeper-idle-retire (Cowboy) | 5 | 5 | 2 | 3 | 2 |
| pr8818 buyer profiles (Suger) | 5 | 5 | 4 | 4 | 3 |
| pr8895 deal suggestions (Suger) | 4 | 4 | 3 | 2 | 3 |
| pr8874 guard (Suger) | 0 | 0 (1 minor, real) | 0 | 0 | 0 |
| **Recall** | 14 | **100%** | **64%** | **64%** | **57%** |
| Findings / refuted | | 52 / 3 | 23 / 0 | 22 / 0 | 17 / 0 |
| Tokens (4 inputs) | | 23.5M | 2.1M | 4.0M | 7.9M |
| Minutes (4 inputs) | | 115 | 20 | 14 | 36 |

- Max's extra recall is concentrated in failure-mode and recovery defects
  (recovery after retirement, executor identity, empty-partner configuration)
  and one cross-organization error-text exposure
  that the cheaper arms did not reach; every arm found the most obvious
  defect of each change.
- Max is noisier in absolute terms (3 refuted of 52) but its precision stays
  high (94%); the Medium arms had no refuted findings.
- Limitations: four inputs selected because a review was warranted;
  one adjudicator family (Claude) per input; serious/not-serious is a
  judgment the adjudicators made; the reference set is the union of what the
  four arms found, so defects every arm missed are not counted.

## Public evidence coverage

Scored lineages per executable target and task (after canonical selection;
saturated groups — GPQA Diamond — are dropped):

| Target | agentic coding | reasoning | long context | code review | tool use |
| --- | --- | --- | --- | --- | --- |
| gpt-6.2-astra:max | 2 | 2 | 0 | 3 (pilot) | 0 |
| gpt-6.2-astra:medium | 1 | 2 | 0 | 3 (pilot) | 0 |
| gpt-6.2-sol:max | 2 | 2 | 0 | 0 | 0 |
| gpt-6.2-sol:medium | 1 | 2 | 0 | 3 (pilot) | 0 |
| gpt-6-luna:max | 2 | 3 | 1 | 0 | 0 |
| claude-fable-5-2:max | 3 | 3 | 1 | 0 | 0 |
| claude-fable-5-2:high | 1 | 3 | 1 | 0 | 0 |
| claude-opus-5-5:high | 1 | 3 | 1 | 0 | 0 |
| claude-sonnet-5-5:xhigh | 1 | 4 | 1 | 0 | 0 |
| claude-sonnet-5-5:medium | 1 | 3 | 1 | 0 | 0 |

Opus 5.5 default, Gemini 3.8 Flash (Antigravity), the three DeepSeek V4.1
Flash bindings and the Grok preset have no observations yet.

Unverified observations from the local inputs, recorded to guide the next
refresh rather than as conclusions:
- Agentic coding at max effort (Terminal-Bench 4.x, two harnesses): GPT-6.2
  Astra, Sol and Claude Fable 5.2 fall within a few points, and their order
  differs between the tbench.ai leaderboard and Artificial Analysis's run
  *(local inputs)*.
- Artificial Analysis cost to run its index and ARC cost per task order the
  same models similarly, Sol and Luna cheapest and Fable max most expensive,
  on different absolute scales *(local inputs)*.
- Opus 5.5 scored higher at high than at max on ARC-AGI-2 with partial
  coverage *(local inputs)*.

## Gaps and next actions

- **Code review:** no public (model, effort) benchmark matches an executable
  target; ReviewBench ranks agents, SWE-PRBench covers older models. Grow the
  verified pilot (more inputs, including low-risk changes, and a Claude
  reviewer arm) before trusting `standard` effort for any class of review.
- **Agentic coding below max:** every below-max preset (Astra/Sol Medium,
  Fable High, Opus High, Sonnet XHigh/Medium) has one lineage; the tbench.ai
  4.1 and ARC rows used here are max or xhigh only. A second independent
  source or internal pilots are needed.
- **Long context:** insufficient rather than absent — at most one lineage
  (AA-LCR) per target, none for GPT-6.2. MRCR is vendor-only.
- **Tool use, writing:** no usable public rows for current targets.
- **Single-preset Providers:** Gemini, DeepSeek and Grok bindings need
  observations under their own harness before any recommendation.
- **Subscription cost:** API-equivalent dollars and tokens are not quota.
  Record per-model quota burn from usage windows before ranking cost across
  models with different rates.
- **Access:** Artificial Analysis tokens-per-task needs a Data API key.
- **Mapping decisions pending:** Scale MCP-Atlas evaluates models in their API
  default configuration; it is recorded on research targets, not on
  `opus-default`.

## Sources (retrieved 2026-11-18)

- Artificial Analysis leaderboard — https://artificialanalysis.ai/leaderboards/models (reference only)
- Terminal-Bench 4.1 — https://www.tbench.ai/leaderboard/terminal-bench/4.1 (reference only; no data licence stated)
- ARC Prize leaderboard — https://arcprize.org/leaderboard (reference only)
- Epoch AI benchmark data — https://epoch.ai/data/benchmark_data.zip (CC-BY 4.0)
- Scale SEAL HLE and MCP-Atlas — https://scale.com/leaderboard (reference only)
- Claude Fable 5.2 and Opus 5.5 system cards — anthropic.com (vendor, reference only)
- GPT-6.2 Astra/Sol system card — deploymentsafety.openai.com (vendor; no rows recorded)
- tau2-bench — https://github.com/sierra-research/tau2-bench (MIT)
- ReviewBench — https://review-bench.ai (not recorded: agent-level entries only)
- Cowboy managed-call ledger and review pilot (internal)
