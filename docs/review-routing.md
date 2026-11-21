# Review routing: tiers, cost and project risk

Status: design for review (2026-11-21). Replaces the editable reviewer family
order and routine effort in Settings → Agent tools.

## Problem

Read-only query of the Hawk Controller ledger, 14 days to 2026-11-21: 538 GPT
reviews and design reviews. 511 ran `astra-max` (about 1.59B tokens, 93% of
all GPT managed-call tokens); median 2.3M tokens and 10 minutes per review,
largest 17M. 98% passed the preset explicitly, typically with the reason
"Astra Max per review_selection".

Causes:

1. **The routine order is strongest-first.** `selection::candidates` sorts by
   family order, then effort; the default `gpt_effort = max` puts
   `astra-max` first for every review. Capabilities publish only this order
   as `review_candidates`, and `risk` changes it only under `standard`.
2. **The caller picks the preset.** The guidance tells agents to use
   `review_candidates` in order; they copy the first ID, so an explicit preset
   bypasses `risk` entirely.
3. **Nothing tells the caller what a choice costs.** The models help lists
   capability scores, not the per-review cost of each preset.
4. **Volume and fan-out.** 38 reviews a day; 166 started within ten minutes of
   the previous review in the same session (fix/re-review loops); Suger
   workflows fan out per aspect and run evidence audits (`speckit.e2e`
   execution auditor, 65 calls) as code reviews.

The same pattern applies to Claude: `fable-max` costs far more than Opus and
is only justified by a concrete reason.

## Evidence

Paired pilot on four real reviews with 14 blind-verified serious defects
(`models/pilots/2026-11-18-review-pilot.json`, Sol Max arm added in
`models/pilots/2026-11-21-sol-max-arm.json`; the Sol Max match is unblinded):

| Preset | Serious found | Tokens (4 inputs) | Minutes |
| --- | --- | --- | --- |
| Astra Max | 14/14 | 23.5M | 116 |
| **Sol Max** | **12/14** | **12.9M** | **29** |
| Astra Medium | 9/14 | 2.1M | 20 |
| Sol Medium | 9/14 | 4.0M | 14 |

Sol Max keeps most of Max's recall (missed one recovery-feasibility and one
status-overwrite defect) at about 55% of the tokens and a quarter of the wall
time. Medium arms miss about a third of serious defects and suit only a
targeted check, such as confirming named fixes.

## Design

### 1. Tiers (Cowboy-owned periodic configuration)

`models/routing.toml` is versioned beside the model evidence, regenerated or
edited by the `model-intel` refresh, reviewed like the catalog, and embedded
in the Controller (like `models/summary.md`). It maps a purpose and tier to an
ordered list of signed preset IDs per Provider, with the cost evidence each
row relies on:

```toml
schema = 1
evidence = "2026-11-21"

[review.codex]
high = ["astra-max"]
routine = ["sol-6-2-max", "astra-max"]
verify = ["astra-medium", "sol-6-2-medium"]

[review.claude-code]
high = ["fable-max", "fable-high"]
routine = ["opus-high", "opus-default"]
verify = ["opus-default"]

[design_review.codex]   # same tiers
[design_review.claude-code]

[cost."codex/astra-max"]
median_tokens = 2300000
median_minutes = 10
serious_recall = "14/14"
```

- **high**: security, permissions, concurrency, persistence, deletion, money,
  large refactors, or a risk area the project names. Requires a one-line
  reason.
- **routine** (default): an ordinary change review.
- **verify**: confirming that named findings were fixed, or a later round of
  the same `task`.
- Later entries in a tier are alternatives when the earlier preset is not
  runnable on the pinned generation or its quota is not sufficient; the
  choice and the reason are recorded. A tier never borrows a preset from a
  higher tier.
- A preset ID absent from the pinned generation is skipped; an empty result
  refuses `preset_unavailable` as today.

### 2. Cost mode (the one user setting)

Settings → Agent tools → Review shows the routing table read-only, with the
evidence date and each row's cost, plus a single **Cost** option:

| Mode | routine uses | verify uses | high uses |
| --- | --- | --- | --- |
| Quality | high row | routine row | high row |
| **Balanced** (default) | routine row | verify row | high row |
| Economy | verify row | verify row | routine row |

The family order and routine-effort controls and their session overrides are
retired. Existing stored values are ignored and kept readable for rollback.

### 3. Project risk (project configuration)

Each repository names what is high risk for it in its own guidance, e.g. a
`## Review risk` section in `AGENTS.md` (paths, subsystems, and what does not
need a high tier). Agents already load that file natively, so no new Cowboy
transport is needed. The Cowboy guidance tells the caller to apply it when it
declares the tier. Automatic path matching against the captured diff is a
possible later phase; it would need the snapshot's file list at admission.

### 4. Admission

- `review.start`/`cowboy review` take `risk: "high" | "routine" | "verify"`;
  omitted means routine. `risk: "high"` without a `reason` is refused
  (`risk_reason_required`).
- A fresh review whose `task` already has a completed call under the same
  parent becomes **verify** unless it declares `high`.
- An explicit `preset` must belong to the resolved tier (after cost mode);
  otherwise `preset_outside_tier`, with the tier's presets in the hint. A
  preset the user explicitly asked for is passed with `risk: "high"` and the
  user's request as its reason; the receipt records it.
- A continuation keeps its child's preset, as today.
- The call records `labels.review_tier`, the cost mode and any alternative
  taken, and the Calls detail shows them.

### 5. What the caller sees

Capabilities replace the routine-only `review_candidates` with:

```json
"review_routing": {
  "cost_mode": "balanced",
  "evidence": "2026-11-21",
  "tiers": {
    "high":    {"presets": ["astra-max"], "median_tokens": 2300000, "relative_cost": 1.0},
    "routine": {"presets": ["sol-6-2-max"], "median_tokens": 1300000, "relative_cost": 0.55},
    "verify":  {"presets": ["astra-medium"], "median_tokens": 200000, "relative_cost": 0.09}
  }
}
```

`review_candidates` stays, equal to the routine tier, for older callers. The
guidance becomes: declare the tier from the project's risk guidance; do not
pass a preset unless the user named one; high needs a reason; a fix-up round
is verify; evidence audits and plan checks are `analysis`, not reviews;
continue the same review child instead of a fresh full review where possible.

## Native coverage and residual gap

Codex and Claude choose their own model only for their own turns; neither
routes a cross-Provider review by risk and cost. The residual gap is Cowboy's
managed review admission, which already owns preset selection; this replaces
its ordering rule and Settings surface and adds no new runtime. Delete the
table when a native reviewer routes by declared risk.

Claude follower classification: shared (the same admission and guidance serve
both Providers).

## Compatibility

Controller-only plus Web. Older callers that pass `astra-max` for an
undeclared review are refused with `preset_outside_tier` and a hint; this is
intentional. Rolling the Controller back restores the old ordering; the
stored cost mode is ignored by older builds.

## Expected effect

On the 14-day mix, if 70% of reviews are routine (Sol Max, ~0.55×), 20% verify
(~0.1×) and 10% high (1×), GPT review tokens fall by about half; moving
evidence audits to analysis and verify rounds removes more.
