+++
schema = "cowboy.pattern/v1"
id = "delivery.follow-through"
revision = 1
role = "supervisor"
summary = "Follow up an already-authorized unfinished delivery using exact evidence."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Follow through on authorized work

Design example; not installed or activated.

## Apply when

The enrolled session has a current, explicit delivery scope; its owning
procedure is available; and authoritative evidence identifies a missing step
within existing authorization. An ended turn is a trigger to check, not proof
that delivery failed or completed.

## Exclude

Research/proposal-only work, paused/stopped/excluded sessions, another task's
artifacts, or a step requiring broader permission. Unknown scope, evidence,
worker ownership or operation outcome does not match automatically.

## Read

Read the accepted scope, selected procedure, current exact commit/environment,
needed check or release receipts, and the original session's queue state. Read
only the history excerpt required to resolve a discrepancy.

## Supervise

Ask the original session once to complete the concrete missing authorized step
through its owning workflow. Attach the evidence references and trusted
coordinator origin. Recheck the current grant, policy version and pause state at
dispatch. Do not interrupt, answer human approvals or edit another worktree.

Observe the resulting evidence. Reuse the same causal identity and original
operation after uncertain delivery; do not manufacture a new request to retry.

## Complete or stop

Complete this intervention only when the required evidence agrees with the
current scope, revision and target. This does not create a separate task status.

Stop automatic attempts on no progress, scope change, missing authorization, an
unknown external effect, or the configured cause budget. Prepare any
independently authorized result before asking one concrete question.

## Counterexamples

- A design session creates a local commit: that does not authorize publication.
- A newer commit exists: a passing check on the old SHA is insufficient.
- A release request timed out: inspect its receipt before another request.
- A session is paused: a later green check does not resume it.

## Evidence basis

This is a candidate supervision rule derived from current delivery contracts,
not a claim of measured autonomous success. Store later outcome references in
their access-controlled source; do not paste transcripts or live counters here.
