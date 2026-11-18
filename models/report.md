# Model evidence report — 2026-11-18

First run of the `model-intel` skill. Scores and tiers are in
`models/catalog.json`; the agent-facing digest is `models/summary.md`.

Inputs: 46 repository observations (`sha256:6afc123d…c04f`) and 164
reference-only observations kept in local state (`sha256:3cfb64c1…01fc`),
of which 177 rows were valid and scored. Numbers from sources whose terms
forbid redistribution are not reproduced here; statements derived from them
are marked *(local inputs)* and can be checked only against that digest.

## What the evidence decides

The catalog determines **no tier for any executable Codex or Claude preset**
yet, and the signed Recommended order and defaults stay unchanged:

- Every GPT-6.2 and Claude Fable 5.2 preset is provisional (released within 14
  days); they are scored for inspection but not ranked until 2026-12-01 and
  2026-11-23 respectively.
- The remaining Claude presets and Luna do not share enough independently
  scored tasks to be ranked against each other (agentic coding below max
  effort has one public lineage each).
- The review pilot below is unverified, so it is reported but not scored.

The one determined tier, `analysis` flagship/workhorse `luna-6-max` for
Codex, is an artifact of Luna being the only non-provisional Codex target; it
should not be used for routing.

## What the pilot suggests (directional, not scored)

Routine Codex reviews probably do not need Max effort. On three historical
inputs with findings, GPT-6.2 Astra Medium and Sol 6.2 Medium agreed with
more of the unverified union of serious findings than the original GPT-6
Astra Max review, using 27% and 50% of its tokens and 55% and 40% of its wall
time respectively. This supports offering an opt-in review-effort choice and
a larger, verified pilot; it does not justify changing the default selector.

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

Four historical Astra Max reviews whose worktrees were still clean and
unchanged were re-run with the original focus text and base on Sol 6.2
Medium and Astra Medium. Reference set: the union of serious findings (high,
or medium with concrete correctness/security/contract impact) across the
three arms, matched by root cause. Findings were **not** verified against the
code, so the figures below measure agreement with an unverified union, and
false positives are unmeasured. Rows:
`models/observations/2026-11-cowboy-pilot.jsonl` (`validity = "unverified"`).

| Input | Union | GPT-6 Astra Max | 6.2 Astra Medium | 6.2 Sol Medium |
| --- | --- | --- | --- | --- |
| keeper-idle-retire (Cowboy) | 6 | 4 | 3 | 4 |
| pr8818 buyer profiles (Suger) | 9 | 3 | 8 | 7 |
| pr8895 deal suggestions (Suger) | 11 | 7 | 9 | 7 |
| pr8874 guard (Suger) | 0 | approve | approve | approve |
| **Agreement with union** | 26 | **54%** | **77%** | **69%** |
| Tokens (4 inputs) | | 7.88M | 2.13M (27%) | 3.96M (50%) |
| Minutes (4 inputs) | | 35.7 | 19.6 (55%) | 14.2 (40%) |

- Each arm found serious issues the others missed. Astra Medium found fewer
  than the baseline on keeper-idle-retire (3 vs 4); the baseline uniquely
  found the managed conversation-continuation recovery gap and the
  Account-only Salesforce link case.
- Sol Medium used more tokens than Astra Medium on all four inputs but
  finished faster; whether it is cheaper in subscription quota depends on the
  per-model quota rate, which no source here measures.
- The pr8895 focus listed prior findings to verify, so part of its agreement
  was prompted.
- Limitations: three scored inputs; unverified union; baseline is GPT-6
  Astra, not 6.2; no GPT-6.2 Astra Max arm.

## Public evidence coverage

Scored lineages per executable target and task (after canonical selection
and after dropping saturated groups: ARC-AGI-2, GPQA Diamond, Scale
MCP-Atlas and the vendor-only SWE-bench Pro):

| Target | agentic coding | reasoning | long context | code review | tool use |
| --- | --- | --- | --- | --- | --- |
| gpt-6.2-astra:max | 2 | 1 | 0 | 0 | 0 |
| gpt-6.2-astra:medium | 1 | 1 | 0 | 0 (3 unverified) | 0 |
| gpt-6.2-sol:max | 2 | 1 | 0 | 0 | 0 |
| gpt-6.2-sol:medium | 1 | 1 | 0 | 0 (3 unverified) | 0 |
| gpt-6-luna:max | 2 | 2 | 1 | 0 | 0 |
| claude-fable-5-2:max | 2 | 2 | 1 | 0 | 0 |
| claude-fable-5-2:high | 1 | 2 | 1 | 0 | 0 |
| claude-opus-5-5:high | 1 | 2 | 1 | 0 | 0 |
| claude-sonnet-5-5:xhigh | 1 | 3 | 1 | 0 | 0 |
| claude-sonnet-5-5:medium | 1 | 2 | 1 | 0 | 0 |

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
  target; ReviewBench ranks agents, SWE-PRBench covers older models. Verify
  the pilot's reference findings, then pair the next ten routine reviews
  (cheaper preset plus the normally selected one).
- **Below-max efforts:** Artificial Analysis lists some medium/high rows (one
  lineage per task); the tbench.ai 4.1 leaderboard and ARC rows used here are
  max or xhigh only. Two independent lineages below max need either another
  public source or internal pilots.
- **Tool use, long context, writing:** no usable public rows for current
  targets; MRCR is vendor-only.
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
