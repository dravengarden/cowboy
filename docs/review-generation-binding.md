# Review generation binding

Status: proposed, not implemented. Closes the "Known gap" recorded in
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
- `claim` re-resolves readiness (unchanged). If the claimed generation
  differs from the one used at admission (for example an update landed while
  the call waited for approval):
  - `selection = auto`: re-rank on the claimed generation with the same
    preferences and risk label and record the new preset in the same claim
    CAS. No new admission, no request conflict.
  - `selection = explicit`: keep the ID only if the claimed generation still
    signs it and it resolves to a reviewer family; otherwise fail
    `preset_unavailable` / `review_model_not_allowed` with no worker start.
- Admission records the ranked generation in the existing
  `provider_version` / `provider_generation_digest` fields as a hint; claim
  overwrites them as today. No ledger schema change.

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
- `ManagedTurn` gains `delivered` (in-flight or a `user_message_chunk` after
  the cursor) distinct from `queued`. When recovery finds a Running call
  whose managed prompt is queued but not delivered, it removes that queued
  entry, runs `configuration::ready` (which also rejects preset-less legacy
  reviews), and resubmits under the same message id. Implementation must
  confirm the Hub never auto-dispatches a queued managed prompt before the
  runner's recovery pass; if it can, the queue holds managed prompts until
  the runner releases them.

### D. Reuse only checked reviews

- Bump review identity to schema 2. Pre-guard sources (schema 1) never match
  a new request, so no completed verdict admitted before the guard is
  reused. Existing reservations simply age out.
- Before `reserve_review_input`, run the pure part of `apply_preset`
  (resolve values on the claimed generation, signed preset, reviewer family)
  so a request that could not run cannot reserve an identity either.
- `reuse_source` additionally requires the source's recorded
  `configuration.model` to be a reviewer family of the Provider; otherwise
  `review_source_not_reusable`.

`apply_preset` is split into `resolve_preset` (pure, returns the value map)
and the preference write, so A, B and D share one validation.

## Compatibility and rollout

- Controller-only release (`.#cowboy-controller-release`); no Machine,
  worker, wire protocol or ledger migration. Review identity schema 2 is a
  digest input change only.
- In-flight calls at upgrade: queued/starting calls follow the new claim
  path; running calls keep their pinned generation; a running call whose
  prompt is still queued gets the recovery check in C.
- Behaviour change visible to users: changing a managed child's model from
  the UI or MCP now fails with an explicit message (it was never honoured
  reliably).

## Tests

- Selection: latest package ≠ runnable generation → auto pick comes from the
  runnable one; explicit ID absent on runnable → refused at admission.
- Claim: generation changed after admission → auto re-ranked in the claim
  CAS; explicit missing → failed, no worker started.
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
