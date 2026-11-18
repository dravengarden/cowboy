# Model evidence report — 2026-11-18

First run of the `model-intel` skill. Scores and tiers are in
`models/catalog.json`; the agent-facing digest is `models/summary.md`.
Numbers from sources whose terms forbid redistribution are kept outside the
repository and appear here only as derived conclusions.

## Decisions the evidence supports

1. **Routine Codex reviews do not need Astra Max.** In a paired pilot on three
   historical inputs with findings, GPT-6.2 Astra Medium and Sol 6.2 Medium
   each found at least as many serious issues as the original GPT-6 Astra Max
   review, at roughly a quarter to a half of its tokens and a third of its
   wall time. This is directional (three inputs, unverified reference set,
   different model generation for the baseline) and justifies an opt-in
   review-effort choice, not an automatic default change.
2. **Keep the current Codex New Session default (Sol 6.2 Medium).** No
   executable Codex target has enough independent evidence for the
   `session_default` workload; the public agentic-coding runs that exist
   place Sol and Astra close together at max effort with Sol cheaper, which is
   consistent with the current default but does not prove it.
3. **No Claude recommendation change.** No Claude preset has two independent
   lineages for agentic coding below max effort, and Fable 5.2 is still in the
   provisional window. Revisit after 2026-11-23.

## Baseline: what managed calls used (Hawk Controller, 2026-10-09 → 11-18)

934 managed calls. Astra Max served 593 reviews, 126 design reviews and 24
analysis calls (about 80% of all calls). Median Astra Max review: 1.8M tokens
(146k uncached input), 7.8 minutes. The 11 Astra Medium reviews in the ledger
had a median of 0.15M tokens and 0.9 minutes, but on unpaired inputs.
Cause: the review selector orders the GPT family Astra first and effort max
first, and takes the first candidate when no preset is passed
(`components/server/src/server/managed_calls/selection.rs`).

## Review pilot (internal evidence)

Four historical Astra Max reviews whose worktrees were still clean and
unchanged were re-run with the original focus text and base on Sol 6.2
Medium and Astra Medium. Reference set: the union of serious findings (high,
or medium with concrete correctness/security/contract impact) across the
three arms, matched by root cause; findings were not independently verified.

| Input | Union | GPT-6 Astra Max | 6.2 Astra Medium | 6.2 Sol Medium |
| --- | --- | --- | --- | --- |
| keeper-idle-retire (Cowboy) | 6 | 4 | 3 | 4 |
| pr8818 buyer profiles (Suger) | 9 | 3 | 8 | 7 |
| pr8895 deal suggestions (Suger) | 11 | 7 | 9 | 7 |
| pr8874 guard (Suger) | 0 | approve | approve | approve |
| **Recall** | 26 | **54%** | **77%** | **69%** |
| Tokens (4 inputs) | | 7.88M | 2.13M | 3.96M |
| Minutes (4 inputs) | | 35.7 | 19.6 | 14.2 |

Observations:
- Each arm found serious issues the others missed, so a single review of any
  preset is not exhaustive. The Max baseline uniquely found the managed
  conversation-continuation recovery gap and the Account-only Salesforce
  link case.
- Sol Medium used more tokens than Astra Medium on all four inputs but
  finished faster; whether it is cheaper in subscription quota depends on the
  per-model quota rate, which no source here measures.
- The pr8895 focus listed prior findings to verify, so part of its recall was
  prompted; only two of its serious findings were new to the Max baseline.

Limitations: three scored inputs; unverified union (false positives are not
measured); baseline is GPT-6 Astra, not 6.2; no GPT-6.2 Astra Max arm, so the
pilot cannot say what Max adds on the current generation.

## Public evidence coverage

177 public observations from primary sources (13 redistributable, 164
reference-only). Lineages per executable target and task:

| Target | agentic coding | reasoning | long context | code review | tool use |
| --- | --- | --- | --- | --- | --- |
| gpt-6.2-astra:max | 2 | 1 | 0 | 0 | 0 |
| gpt-6.2-astra:medium | 1 | 1 | 0 | 3 (pilot) | 0 |
| gpt-6.2-sol:max | 2 | 1 | 0 | 0 | 0 |
| gpt-6.2-sol:medium | 1 | 1 | 0 | 3 (pilot) | 0 |
| gpt-6-luna:max | 2 | 2 | 1 | 0 | 0 |
| claude-fable-5-2:max | 2 | 2 | 1 | 0 | 0 |
| claude-opus-5-5:high | 1 | 2 | 1 | 0 | 0 |
| claude-sonnet-5-5:xhigh | 1 | 3 | 1 | 0 | 0 |

(Counts after the scorer dropped saturated groups: ARC-AGI-2 and GPQA
Diamond, whose best scores exceed 0.9, and the vendor-only SWE-bench Pro.)

Qualitative findings, without republishing restricted numbers:
- Agentic coding at max effort (Terminal-Bench 4.x, two harnesses): GPT-6.2
  Astra, Sol and Claude Fable 5.2 are within a few points of each other, and
  their order flips between the tbench.ai leaderboard and Artificial
  Analysis's run. Treat the frontier as tied on this axis.
- Cost to run Artificial Analysis's index and ARC cost per task agree on the
  order: Sol and Luna cheapest, Fable max most expensive.
- Effort scaling is not monotonic at the top: Opus 5.5 scored higher at high
  than at max on ARC-AGI-2 (partial coverage).

## Gaps and next actions

- **Code review:** no public (model, effort) benchmark matches an executable
  target; ReviewBench ranks agents, SWE-PRBench covers older models. Grow the
  internal pilot: pair the next ten routine reviews (cheap preset plus the
  normally selected one) and verify the findings.
- **Tool use, long context, writing:** no usable public rows for current
  targets; MRCR is vendor-only.
- **Below-max efforts:** public leaderboards publish max effort only, so
  medium/high presets rely on internal evidence.
- **Subscription cost:** API-equivalent dollars and tokens are not quota.
  Record per-model quota burn from usage windows before using cost to rank
  across models with different rates.
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
