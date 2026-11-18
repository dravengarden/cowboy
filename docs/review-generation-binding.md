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
- An explicit review `--preset` is checked against the runnable
  generation's reviewer candidates, not the latest Catalog.
- `claim` re-resolves readiness (unchanged) and, for a review, resolves the
  admitted preset on the generation it pins. If an update landed while the
  call waited (for example for approval) and the ID is missing or no longer a
  reviewer family, the call fails `preset_unavailable` /
  `review_model_not_allowed` before any snapshot or worker; the caller
  submits again and admission ranks the new generation.
- Claim does not silently re-rank an automatic choice. The ledger `Record`
  denies unknown fields, so recording whether the preset was automatic would
  make a rolled-back Controller unable to read new records; the rare
  update-during-wait case fails visibly instead.

### B. Freeze a managed child's generation

- `repin_workerless_provider` refuses managed children, matching the live
  reload refusal. This covers both dormant and missing-generation re-pin.
  A managed child whose generation was uninstalled cannot continue; the
  continuation fails `conversation_unavailable` and the caller starts a
  fresh call.
- Defense in depth: `configuration::ready` fails `generation_changed` when
  the child's `meta.provider_version/digest` differs from the record's
  claimed values, before `start_registered_session`.

### C. Only the runner configures a managed child

- `config_revert::apply` refuses managed children for every client and Agent
  API entry point (`managed child configuration is owned by its call`). The
  runner keeps writing preferences through `apply_preset`.
- The Hub persists queues, so a managed prompt queued before a Controller
  restart would otherwise drain as soon as the child is ready. The queue now
  dispatches a managed prompt only after the runner released it in this
  process (`submit_managed_prompt` releases after `configuration::ready`;
  the release set is in memory only). `ManagedTurn::held` reports a queued,
  unreleased prompt; the runner then runs `configuration::ready` again
  (which also rejects preset-less legacy reviews) and calls
  `release_managed_prompt`, or discards the prompt and fails the call. A
  finished call discards its prompt and forgets its release.

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

## Tests

- Selection: latest package ≠ runnable generation → auto pick comes from the
  runnable one; explicit ID absent on runnable → refused at admission.
- Claim: preset no longer a reviewer on the pinned generation → failed, no
  snapshot or worker.
- Re-pin: dormant and missing-generation re-pin refuse a managed child;
  `ready` fails `generation_changed` on a hand-modified child meta.
- Configure: `SetConfigOption` / `sessions.configure` on a managed child is
  refused; ordinary sessions unchanged.
- Recovery: Running call with queued-only prompt runs `ready` before
  resubmit; preset-less legacy review fails `review_model_not_allowed`.
- Reuse: schema-1 source never matches; source with non-reviewer recorded
  model is not reused; disallowed request reserves nothing.

## Non-goals

- Quota-aware routing or automatic fallback between families (unchanged:
  the caller decides).
- Per-repository scenario → model routing (waits for the 12-01 catalog
  refresh).
