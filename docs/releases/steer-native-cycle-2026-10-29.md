# Steering a cycle Claude starts on its own — 2026-10-29

Status: source accepted; deployment receipts below.

## Problem

Claude resumes on its own after a background task ends. ACP runs that as
foreground work that no `session/prompt` owns, so Cowboy minted no turn for
it. The session was Busy, the delivery picker had no target and showed Steer
and Next checkpoint as unavailable, and a message could only wait for the
cycle to end. Observed in session `sess-1793250451736`: a turn ended at
10:45:43 UTC, Claude started its follow-up at 10:50:51 with no user message,
and the picker opened at 10:51 offered no Steer.

## Design

The identity of such a cycle belongs to the adapter that runs it.

- `plugins/claude-code/runtime/native-cycle.mjs` names each foreground work
  that starts as ACP's `AUTONOMOUS_CYCLE` once (`native-<uuid>`) and announces
  the name and its end as a `_claude/sdkMessage` system frame with subtype
  `cowboy_native_cycle`. A turn that takes the work over keeps the name; the
  work's end ends it. An older worker ignores the unknown subtype.
- The worker records the name and emits the `cowboy_native_cycle` session
  update. Like `cowboy_background_tasks` it is session metadata: never a
  transcript row and never a turn edge, so the Controller's dispatch guard,
  queue and turn completion are untouched. `WorkerSnapshot.native_cycle_id`
  restores it after a reconnect; older peers omit and ignore the field.
- `WorkerSnapshot::delivery_turn_id` is the Cowboy turn, or else the named
  cycle. The worker, Machine broker and Controller use it for current-turn
  delivery targets and admission. Interrupt and cancellation still require a
  Cowboy turn. A Cowboy `TurnStarted` clears the name.
- A steer addressed to the name is checked by the worker when it arrives and
  again under the steering gate (no prompt, same cycle), then by the adapter in
  one synchronous section: only the same running work is joined, through
  ACP's own takeover of a self-started cycle by a new turn, carrying the
  requested `now`/`next` priority (`later` while a question is open). Should
  the cycle end during ACP's admission awaits, the message is queued as an
  ordinary next turn and never enters a different cycle. An ended, replaced or
  foreign cycle is refused before anything reaches Claude; the sender keeps
  the message.
- The picker labels the joined Steer "Joins the agent's follow-up now", and
  Next checkpoint now reports the actual reason it is unavailable.

A first, Controller-only attempt dispatched a prompt from the Hub's Busy
state. Review found that it had no per-cycle identity, lost the target after
Hub admission, could release the dispatch guard on the previous cycle's idle
edge and acknowledged delivery before the payload was durable. It was
discarded.

## Verification

| Gate | Result |
| --- | --- |
| Adapter unit tests (`plugins/claude-code/runtime/*.test.mjs`) | 120 passed, 7 new |
| Rust (wire, worker, Controller projection, ACP session conformance) | 12 targeted tests passed, 4 new |
| `just claude-remote-check` | 358 + 4 passed |
| Claude worker conformance, all twelve phases, packaged adapter | Accepted, 96 checks: the previous 95 plus `packaged_steer_joins_exactly_the_named_self_started_cycle` |
| Claude managed / Codex worker / session conformance | Accepted |
| Delivery picker browser conformance | Passed |

The new packaged check uses the real Claude Code 2.1.312 and ACP 0.99.0: a
background command ends a prompt turn, its completion starts Claude's own
cycle, the adapter names it, a foreign name is refused, the named steer is
injected and reaches the model inside that cycle, the cycle's end is
announced, and a steer to the ended name is refused.

## Delivery

Order: Controller and Web (they consume and filter the update), the Claude
Code Plugin (the adapter that names cycles), then OVH worker maintenance (the
worker and broker that carry and admit the name). Until a session runs both
the new Plugin and the new worker, its self-started cycles stay unsteerable as
before.
