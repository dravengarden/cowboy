# Review routing: tiers, cost and project risk

Status: implemented and active on Hawk (Controller and Web `7a821ea64`,
2026-11-21); the read-only Settings table and field costs followed. Revision 1
was reviewed by Sol Max (`call-e1837ba3d988af644704918d`, 16 findings);
§Revision 1 findings lists how each was resolved.

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

Paired pilot on four real reviews; 14 serious defects were blind-verified on
2026-11-18 (`models/pilots/2026-11-18-review-pilot.json`). The Sol Max arm
(`models/pilots/2026-11-21-sol-max-arm.json`) was matched against that set by
the orchestrating agent, unblinded, and its new findings were not
adjudicated:

| Preset | Matched known serious | Tokens (4 inputs) | Minutes |
| --- | --- | --- | --- |
| Astra Max (GPT-6.2) | 14/14 | 23.5M | 116 |
| Sol Max (GPT-6.2) | 12/14 | 12.9M | 29 |
| Astra Medium | 9/14 | 2.1M | 20 |
| Sol Medium | 9/14 | 4.0M | 14 |

Only three inputs contained serious defects, the reference set cannot count
defects every original arm missed, and the scorer excludes the unverified
rows. This supports trying Sol Max as the routine default and measuring the
result, not a fleet-wide quality claim. There is no Claude reviewer arm: Opus
versus Fable cost and recall are unknown here.

## Design

Revision 2 changes only preset ordering, eligibility, published evidence and
guidance. It adds no wire value, label, stored field or Settings schema, so a
Controller rollback reads every request and preference it wrote.

### 1. Routing table (Cowboy-owned periodic configuration)

`models/routing.json` is versioned beside the model evidence, maintained by
the `model-intel` refresh, reviewed like the catalog and embedded in the
Controller. Each row names a reviewer by its **resolved model and effort**,
not a preset ID, because one signed ID (for example `astra-max`) resolves to
different models on different generations:

```toml
schema = 1
evidence = "2026-11-21"
# Applies to review and design_review alike.

[[tier.codex.high]]
model = "gpt-6.2-astra"
effort = "max"
[[tier.codex.routine]]
model = "gpt-6.2-sol"
effort = "max"

[[tier.claude-code.high]]
model = "opus"            # baseline until a paired Claude pilot exists
effort = "high"
[[tier.claude-code.high]]
model = "fable"            # escalation: allowed, never chosen automatically
effort = "max"
explicit_only = true
[[tier.claude-code.routine]]
model = "opus"
effort = "high"
```

A row matches a runnable preset of the pinned generation whose recorded
`model`/`reasoning_effort` (or `effort`) values equal it, with the existing
family normalization (`opus[1m]`). No match leaves that row out; if a tier
has no match, the Controller falls back to today's family/effort order for
that Provider, so a generation the table does not know keeps working.

A Provider without a row (for example Antigravity, once it becomes a call
target) uses today's order until evidence exists.

### 2. Strength order and eligibility

Every reviewer preset gets a **strength rank** within its Provider: the
table's high rows, then routine rows, then the remaining reviewer presets in
today's family/effort order. For a fresh review:

- **Automatic choice** (no `preset`): the first matching row of the declared
  tier (`risk: "high"` → high, otherwise routine), skipping `explicit_only`.
- **Explicit preset**: accepted when its strength is **at or below** the
  declared tier's first automatic choice. A cheaper preset is always allowed
  (for example `astra-medium` to confirm named fixes). A stronger one needs
  `risk: "high"`; otherwise the call is refused `preset_outside_tier` with the
  tier's automatic choice in the hint. Under the Balanced default this
  refuses routine `astra-max` and `fable-max`.
- `risk: "high"` with a blank or defaulted reason is refused
  (`risk_reason_required`). The check runs in shared admission on the
  caller-supplied reason, not on the CLI's filler.
- The Settings family lists still **filter** eligibility (`Opus only`,
  `SOL only` keep excluding the other family) and order alternatives within a
  tier. Claim keeps its existing recheck against them.
- A continuation keeps its child's preset; changing strength needs a fresh
  call. Each continued round still has a tier: the risk it declares
  (`calls.start` accepts `risk` for a review or design review), and an
  undeclared fix-confirmation round is routine. A child whose preset is a
  known reviewer stronger than that tier is refused `preset_outside_tier`;
  the hint names the tier's choice, and a fresh review with the same task and
  base reuses the captured input. `risk: "high"` with a reason keeps the
  strong child. Custom or unrouted presets keep running. (Before this rule,
  one routine review continued on Astra Max for 16 rounds on 2026-11-21;
  18 of that day's 22 Astra Max runs after routing shipped were
  continuations.)
- A review child takes at most three completed rounds. A further
  continuation, at any tier, is refused `review_rounds_exhausted`: findings
  that survive three rounds call for redesigning the mechanism, then a fresh
  routine review of the redesign. Failed or cancelled rounds do not count.
  `cowboy.review.routed` records the child's completed `rounds` on every
  continuation decision.
- Quota stays advisory evidence for the caller. There is no quota-based
  automatic fallback.

The chosen preset is already recorded on the call; generation binding, claim,
recovery and reuse (`review-generation-binding.md`) are unchanged.

### 3. Cost option

The existing per-kind **routine effort** setting becomes the cost option,
with the same stored values: `max` is shown as **Balanced** (the table above)
and `standard` as **Economy** (routine starts at the medium/default effort of
the routine family; high is unchanged). Its description states the measured
trade-off. A Quality option (routine at the high tier) is not added; a caller
gets it per review by declaring high risk. Settings → Agent tools shows the
effective table per Provider, read-only, with its evidence date.

### 4. Project risk (project configuration)

Each repository names what is high risk for it in its own guidance (a
`## Review risk` section in `AGENTS.md`/`CLAUDE.md`): paths, subsystems and
what does not need high. The caller reads the guidance **of the review root**
and the affected paths, not only its own workspace, because a Desktop or
cross-repository review can target another root and a local Claude does not
load `AGENTS.md` natively. Automatic path matching against the captured diff
is a later phase.

### 5. What the caller sees

Capabilities add, per Provider, `review_routing` with each tier's automatic
preset and measured cost (`n`, statistic, source, date; `unknown` where not
measured), and keep `review_candidates` equal to the routine order for older
callers:

```json
"review_routing": {"evidence": "2026-11-21", "cost_option": "balanced",
  "tiers": {
    "high":    {"preset": "astra-max",   "tokens": {"median": 6070000, "n": 4, "source": "pilot"}},
    "routine": {"preset": "sol-6-2-max", "tokens": {"median": 3220000, "n": 4, "source": "pilot"}}},
  "cheaper": [{"preset": "astra-medium", "tokens": {"median": 580000, "n": 4, "source": "pilot"}}]}
```

Token counts are not billing or subscription quota; the guidance says so.
The guidance becomes: declare the tier from the review root's risk guidance;
omit `preset` unless the user named one; a stronger preset needs high risk and
a reason; confirm named fixes with a cheaper explicit preset; keep factual
evidence audits and test-run audits as `analysis`, but design and contract
checks stay design reviews.

### 6. Measured rollout

The ledger already records preset, risk and usage. After release, compare
weekly tokens by tier and re-run a blind pilot on a sample of routine reviews
before tightening further.

## Same vendor, never the same model

A caller may use its own vendor after the others (Auto tries other vendors
first; a session whose targets are all off still calls nothing — the own
vendor is never a way around the switch). A review excludes the reviewer
family of the caller's current model, read from its live configuration, on
**every** Provider, because another vendor's package can run the same model.
A known non-reviewer caller model (Sonnet) excludes nothing; an unknown one
offers no same-vendor reviewer. When the routine tier's own row is excluded
the strongest preset the tier still admits is chosen (a Sol caller gets Astra
Medium); the high tier never falls back automatically, so Fable still has to
be named. Nothing left is refused `same_model`. The claim rechecks the parent's
live model for fresh reviews and continuations (a hibernated parent keeps
admission's check). Capabilities mark the Provider `same_vendor: true`, report
`review_available` apart from analysis availability, and publish its routing
without the caller's model.

Known limit: Auto commits to the first ready Provider before tier
eligibility, so a ready Provider with no eligible reviewer refuses instead of
trying the next one; pass the provider explicitly in that case.

## Logs and metrics

Every routing decision and every finished review leaves a journal record
(Controller stderr → journald → VictoriaLogs), counts and identifiers only:

| Event | When | Fields |
| --- | --- | --- |
| `cowboy.review.routed` | admission picked or refused a reviewer preset | `request_id`, `purpose`, `provider`, `same_vendor`, `risk`, `explicit_preset`, `preset`, `outcome` (`automatic`, `explicit`, `continued` for an admitted continuation round, or the refusal code; a continuation logs its child's preset) |
| `cowboy.review.finished` | a review or design review reached a terminal state | `call_id`, `parent_session_id`, `provider`, `purpose`, `state`, `error`, `preset`, `model`, `effort`, `risk`, `reused`, token counts (total, input, cached, uncached, output, reasoning), `duration_ms`, `verdict`, `review_complete`, `findings` and per-severity counts |

Measurements a worker did not report are omitted, not zero; verdicts other
than the three standard ones are logged as `other`. `cowboy.review.routed` is
a decision record: a duplicate submission of one request can log twice, and a
named preset absent from the generation is refused earlier
(`preset_unavailable`) without this event.

The ledger keeps the full record; `models/field/` aggregates it with
`.agents/skills/model-intel/scripts/field.ts`. VictoriaLogs
(`http://192.168.0.96:6302`) stores the raw line; unpack it with logfmt:

```text
_time:7d _stream:{_SYSTEMD_UNIT="cowboy.service"} "cowboy.review.routed" | unpack_logfmt | stats by (provider, risk, outcome) count() calls
_time:7d _stream:{_SYSTEMD_UNIT="cowboy.service"} "cowboy.review.finished" | unpack_logfmt | stats by (model, effort, risk) count() reviews, median(total_tokens) tokens, median(duration_ms) ms, avg(findings) findings
```

## Native coverage and residual gap

Codex and Claude choose their own model only for their own turns; neither
routes a cross-Provider review by risk and cost. The residual gap is Cowboy's
managed review admission, which already owns preset selection; this replaces
its ordering rule and Settings surface and adds no new runtime. Delete the
table when a native reviewer routes by declared risk.

Claude follower classification: shared (the same admission and guidance serve
both Providers).

## Compatibility

Controller and Web only; no wire, label or storage change. Callers that pass
`astra-max` or `fable-max` for an undeclared review are refused with a hint
(this is the intended behavior change); the bundled review guidance and
`cowboy review` defaults move with the same release. Rolling back restores
today's order.

## Expected effect

On the 14-day mix, moving routine reviews from Astra Max to Sol Max saves
roughly 45% of their tokens on the pilot inputs; cheaper explicit fix checks
and moving evidence audits to analysis save more. The rollout measurement
replaces this estimate.

## Revision 1 findings

| # | Finding | Resolution |
| --- | --- | --- |
| 1 | `verify` breaks rollback | No new risk value; fix checks use a cheaper explicit preset |
| 2 | Task reuse is no fix scope | No automatic verify |
| 3 | Callers miss the review root's guidance | Guidance names the review root; Claude reads it explicitly |
| 4 | Economy weakens high | Economy changes routine only |
| 5 | Routine row admitted Astra Max | Strength rule refuses stronger-than-tier presets |
| 6 | Preset ID ≠ model | Rows match resolved model and effort |
| 7 | Family exclusions lost | Family lists kept as filters |
| 8 | Cost storage breaks rollback | Reuses the stored effort values |
| 9 | Label budget | No new labels |
| 10 | Named-preset exception | Cheaper always allowed; stronger needs high + reason |
| 11 | Continuation strength | Unchanged; fresh call to change strength |
| 12 | Design review route | One table for both purposes |
| 13 | Quota sufficiency | Advisory only, no automatic fallback |
| 14 | Pilot over-claimed | Evidence section restated |
| 15 | Fable default without evidence | Opus is the Claude baseline; Fable explicit-only |
| 16 | Mixed cost populations | Pilot medians with n and source; quota ≠ tokens |

## Appendix: draft Review risk section for Suger

Suger owns its repository guidance; this is a proposal for its `AGENTS.md`,
not applied from Cowboy.

```markdown
## Review risk

- high: tenant isolation and authorization (organization, partner, buyer
  scoping; JWT scopes), PII in responses and logs, entitlement and billing
  sync, database writers and migrations, CRM/marketplace pushes that write
  external systems, Temporal workflow identity and retries.
- routine: console/React presentation, copy, tests only, docs, generated SDKs
  already covered by their generator.
- Speckit aspect fan-out: one fresh routine review per aspect is enough; use
  high only for the aspects above. E2E execution audits and plan-quality
  checks are `analysis` calls, not reviews.
```
