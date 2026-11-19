# Review generation binding

Status: implemented. Closes the "Known gap" recorded in
[`managed-agent-calls.md`](managed-agent-calls.md) after the reviewer
allowlist release (`dd3bc506b`).

## Problem

A managed Review/Design Review must run on a signed reviewer preset (Astra/SOL
for Codex, Opus/Fable for Claude). Today the execution check in
`runner/configuration.rs` enforces the *family* when the worker starts, but
four paths still let the selected preset and the executed generation diverge:

1. **Selection ranks the wrong catalog.** Fresh admission
   (`coordinator.rs`, `selection::candidates`) and capability discovery
   (`review_candidates`) rank `agent_tools::presets`, i.e. the *latest*
   Catalog package. `claim` later pins the generation that the runtime's
   readiness reports, and `apply_preset` resolves the preset ID against that
   generation. If the two differ, an auto-selected ID may be missing
   (`preset_unavailable`), map to a non-reviewer model (fails the guard), or
   map to a lower preference than another ID on the executed generation
   (silent preference skip).
2. **Re-pinning moves a managed child.** Live reloads refuse managed children
   (`hub_provider_pins.rs`), but dormant re-pin (`dormant_pins.rs`) and
   missing-generation re-pin (`session_provider_updates::repin_missing`) do
   not. A hibernated child of a continued call can therefore start a
   different generation than `record.provider_generation_digest`, while
   `apply_preset` resolves values from the recorded one.
3. **Configuration can change between confirmation and delivery.**
   `config_revert::apply` (Web/Desktop `SetConfigOption`, MCP
   `sessions.configure`) does not refuse managed children. In addition,
   `Hub::managed_turn` reports `prompted` while the managed prompt is only
   *queued*, so Controller recovery skips `configuration::ready` and the
   queued prompt can reach a worker that never confirmed the preset
   (including a legacy review admitted without a preset).
4. **Reuse trusts pre-guard results.** `reuse_review` and
   `launch_source` return a completed source verdict before any reviewer
   check. Review identity (`wire/managed_calls/review.rs`, schema 1) binds
   generation and preset ID, so a source admitted before the guard can hand
   a duplicate a verdict that a fresh run would refuse.

The live guard already prevents a non-reviewer model from *executing* a new
review. The remaining risk is preference deviation, avoidable failures, and
reuse of unchecked verdicts.

## Invariant

> A review call decides its Provider generation once, ranks and validates its
> preset against that generation, and every later step (claim, worker start,
> prompt delivery, recovery, reuse) either uses exactly that generation and
> those values or fails before any model tokens are spent.

## Design

### A. Select from the runnable generation

- `first_ready` returns the readiness `generation` together with the target.
  Fresh review admission ranks `selection::candidates` over
  `provider_catalog.package(agent, version, digest)` for that generation;
  an explicit `--preset` is validated against the same package
  (`agent_tools::preset_exists` stays for non-review calls).
- Capability discovery computes `presets` and `review_candidates` from the
  readiness generation it already resolves per Provider, so what an agent
  sees is what admission will choose. Falls back to the latest package only
  when the Provider is unavailable (the call would be refused anyway).
- Every explicit `--preset` (review, analysis or image) must exist on the
  runnable generation (`preset_unavailable` otherwise); a review's must then
  also be one of that generation's reviewer candidates
  (`review_model_not_allowed`).
- A fresh review records the generation its preset was chosen on in the
  existing `provider_version` / `provider_generation_digest` fields at
  admission (no new ledger field, so a rolled-back Controller still reads
  every record). If readiness at claim reports another digest (for example
  an update landed while the call waited for approval), the call fails
  `generation_changed` before any snapshot or worker; an id that a release
  remapped to another model or effort can therefore never run under the
  admitted receipt. The caller submits again and admission ranks the new
  generation. Claim does not silently re-rank.
- Claim also resolves the preset on the pinned generation: it must still be
  a signed reviewer preset and, for a fresh review, a family the session's
  review preferences list (`preset_unavailable` /
  `review_model_not_allowed`). A continuation keeps its child's preset and
  generation, as admission does.

### B. Freeze a managed child's generation

- `repin_workerless_provider` refuses managed children, matching the live
  reload refusal. This covers both dormant and missing-generation re-pin.
  A managed child whose generation was uninstalled cannot continue; its next
  round fails to start (`provider_startup_failed`). The missing-generation
  note is cleared for managed children so it does not expedite update
  passes; it is process-local and could go stale, so claim does not read it.
- Defense in depth: `configuration::ready` fails `generation_changed` when
  the child's `meta.provider_version/digest` differs from the record's
  claimed values, before `start_registered_session`.

### C. Only the runner configures a managed child

- `config_revert::apply` refuses managed children for every client and Agent
  API entry point (`managed child configuration is owned by its call`); MCP
  `sessions.configure` returns a definite `rejected` receipt carrying
  `managed_child_configuration` (never an unknown outcome), and clients
  cannot edit a managed child's queued prompt (`Hub::client_edit`): the
  prompt is the call's durable request and its reusable review identity. The runner keeps writing preferences through
  `apply_preset`.
- The Hub persists queues, so a managed prompt queued before a Controller
  restart drains when its child is ready. With B and the refusal above, that
  worker runs the same generation and replays the same runner-written
  preferences the previous process confirmed, so no new gate is needed. The
  runner withdraws a still-queued prompt that must never start: a review
  that would fail the reviewer check now, such as one admitted without a
  signed reviewer preset (fails `review_model_not_allowed`), and a call
  whose cancellation was accepted (Running → Stopping → Cancelled at once,
  so resuming the queue during the stop grace cannot run it). A finished call
  also withdraws any leftover prompt.
- A refused continuation's child is hibernated, never deleted: deletion
  would tombstone the retained conversation on a Machine with the deletion
  writer. Fresh children are still deleted. The decision uses the request
  the caller already loaded, never a fallible re-read.
- An earlier revision held every queued managed prompt behind an in-memory
  release set. Review showed it could strand a prompt (dispatch requeue,
  ordinary sessions using the prefix, a predecessor's orphaned row) or let a
  replacement worker inherit a release, so it was dropped for the narrower
  rules above.

### D. Reuse only checked reviews

- Bump review identity to schema 2. Pre-guard sources (schema 1) never match
  a new request, so no completed verdict admitted before the guard is
  reused. Existing reservations simply age out.
- Before `reserve_review_input`, run the pure part of `apply_preset`
  (resolve values on the claimed generation, signed preset, reviewer family)
  so a request that could not run cannot reserve an identity either.
- Reservation recovery (`launch_source`) runs the same request check in
  `reuse_source`, which additionally requires the source's recorded
  `configuration.model` to be a reviewer family of the Provider and its preset
  to be signed (not `custom:`); otherwise `review_source_not_reusable`.

`apply_preset` is split into `resolve_preset` (pure, returns the value map)
and the preference write, so A, B and D share one validation.

## Compatibility and rollout

- Controller-only release (`.#cowboy-controller-release`); no Machine,
  worker, wire protocol or ledger migration. Review identity schema 2 is a
  digest input change only. Rolling back the Controller is safe: no record
  gains a field, and the old Controller treats a held prompt as ordinary
  queued work.
- In-flight calls at upgrade: queued/starting calls follow the new claim
  path; running calls keep their pinned generation; a running call whose
  prompt is still queued gets the recovery check in C.
- Behaviour change visible to users: changing a managed child's model from
  the UI or MCP now fails with an explicit message (it was never honoured
  reliably).

## Residual, deliberately not changed

- A reconnecting worker can still settle a managed child on its own
  surviving release (existing runtime reconciliation). The call then pins and
  resolves presets on that same generation, so one call still uses one
  generation.
- A schema-1 reservation already mapped before the upgrade is followed by
  reservation recovery; `reviewed_by_reviewer` still refuses a source without
  a signed reviewer preset and recorded reviewer model. A refused source
  keeps owning its identity; pass `fresh:true` to run a new review.
- `configuration::ready` keeps its existing preference/observation
  handshake; it is not rewritten here.
- Queue and delivery layer. Closed since: cancellation withdraws a
  still-queued prompt as soon as its ledger write commits and, since that
  withdrawal proves no worker saw it, finishes the call Running → Stopping →
  Cancelled at once (only positive withdrawal evidence counts: a missing
  echo is no proof, because the bounded hot log can evict a finished turn's
  echo); every terminal runner transition withdraws a leftover prompt before
  its ledger write; and because that withdrawal is persisted best-effort,
  Controller start removes restored managed rows whose call has ended (before
  Machines reconnect and again after the restore delay). Still open: a
  Controller crash after Machine acceptance can restore a row for work
  already accepted (needs Machine-side delivery evidence keyed by the call's
  message id); a queued row drained by a replacement worker relies on its
  replay of the runner-written preferences; a cancellation that commits
  between launch recording Running and submitting the prompt falls back to
  the ordinary native stop; and a running call whose prompt was requeued (a
  full dispatch channel) or whose worker exited before delivery is not
  retried automatically. An automatic redelivery path was tried and dropped:
  it could fail calls whose Machine reconnects late, wake paused or trashed
  children in a loop, and retry permanent refusals every poll.
- A refused continuation whose worker is still starting cannot be
  hibernated yet; idle hibernation reclaims it later.
- Capabilities publish presets from the read-only profile's generation. An
  image call whose image profile runs elsewhere validates against that
  image generation, so an advertised id can be refused there.
- The ACP configuration bridge of a child agent is not changed; managed
  children run read-only profiles.

## Tests

- Selection: latest package ≠ runnable generation → auto pick comes from the
  runnable one; explicit ID absent on runnable → refused at admission.
- Claim: preset no longer a reviewer on the pinned generation → failed, no
  snapshot or worker.
- Re-pin: dormant and missing-generation re-pin refuse a managed child;
  `ready` fails `generation_changed` on a hand-modified child meta.
- Configure: `SetConfigOption` / `sessions.configure` on a managed child is
  refused; ordinary sessions unchanged.
- Withdrawal: a queued managed prompt is withdrawn by its own call id only,
  and the queue is then empty.
- Reuse: schema-1 source never matches; source with non-reviewer recorded
  model is not reused; disallowed request reserves nothing.

## Non-goals

- Quota-aware routing or automatic fallback between families (unchanged:
  the caller decides).
- Per-repository scenario → model routing (waits for the 12-01 catalog
  refresh).
