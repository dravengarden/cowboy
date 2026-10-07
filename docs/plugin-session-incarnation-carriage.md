# Carrying the Session lineage to the Controller — implemented, held off main

Fourth step of the [durable incarnation design](plugin-session-incarnation-design.md),
after the [reader](plugin-session-incarnation-reader.md) and the
[writer](plugin-session-incarnation-writer.md), which is active on Hawk. The change
is written, tested and **deliberately not on `main`**: it is the local branch
`incarnation-carriage-20261006` (gated at `eeb37e5e`: two carriage commits on `477768bd` merged with `main` at `f9421bc1`; now `6a38c580`, checked out at `worktrees/incarnation-carriage-20261006`) in
the Cowboy repository that backs the task worktrees. It has not been pushed because
pushing a remote branch needs review, and merging it would harm other work (below).

## Behaviour

- **Wire.** `WorkerSnapshot` gains an optional `incarnation` string, omitted when
  absent. Decoding ignores unknown fields in both directions, so a Controller that
  predates it reads a stamped snapshot and an older Machine simply sends none.
- **Machine.** Every snapshot sent to the Controller (individually or in the
  Welcome/resync list) is stamped with the Session's lineage, and only while this
  build's incarnation writer is admitted. A reader-only build holds the same record
  but reports nothing, because it could miss a rotation made by a writer elsewhere.
  A value supplied by a worker is overwritten, never trusted.
- **Controller.** `apply_snapshot` records the reported lineage on the Session. If
  it changes, including a Machine starting or stopping to report one, the Session
  gets a new process-local observation lifetime, so every earlier observation (read
  cache entry, page or diff continuation, buffer-apply authorization) is stale and
  is refused (HTTP 410 for buffered reads) rather than updated in place. Replays of
  the same lineage (every later snapshot and reconnect) change nothing. The first
  value a Machine reports retires existing observations once.

## Findings while writing it

- A first version compared only the value. A lineage that changed `a` to `b` and
  back to `a` made an observation from `a` current again. The Machine never reuses
  a value, but a restored older dataset could report one, so the Controller now
  renews the lifetime on every change as well; a test fails without that.
- Adding the value to `SessionCodeScope` tripped Clippy's large-variant limit on
  `CodeReadScope`. Because the lifetime renewal already decides equality, the
  duplicate field was dropped instead of boxing 29 call sites.

## Evidence

Tests cover the wire both ways, stamping only by an admitted writer (and overwriting a
forged value), the scope semantics above, ingestion of snapshots through the real frame
handler, and the two existing ABA consumers (cached diff pages and buffer-apply
authorization) gaining a lineage variant. The branch, merged with the then-current `main`
(`f9421bc1`) as head `eeb37e5e`, passed the full gate on that exact commit: Rustfmt,
both Clippy configurations, 621 standalone and 1962 all-features tests.

## Why it is held

`WorkerSnapshot` lives in `src/runtime_wire.rs`, one of the files the host-only
Machine releases compare byte for byte against the retained worker bundle
(`retained-worker-interface-compatible`). With this change
`cowboy-machine-writer-host-release` and `cowboy-machine-host-release` refuse to
build: confirmed. Shipping it needs the worker pin to advance, which changes the
worker generation and makes the Machine drain every live worker (replaced through
native resume). That maintenance was not authorized for this work. On `main` it would
also block every other task's resident-only Machine fix until someone advances the
pin. So it waits for a pin advance that is happening anyway, and then needs a
Controller release plus a Machine release that carries the new pin.

Until then the lineage reaches nothing: the Controller learns no value and no
observation is fenced by it.

## Update after the next pin advance

Another task advanced the pin again (`551474a3`, `5b3547a6`, generation
`worker-d62183a8…`) on 2026-10-06. The branch was merged with that `main` as head
`6a38c580` without conflicts, and the host-only writer release still refuses to build
because `runtime_wire.rs` differs from the retained bundle, as expected. That head was
not re-gated; the gate result above is for `eeb37e5e`. The carriage still needs a pin
that includes it.

## Landed

On 2026-10-07 the carriage (`abf08c88`, merged into `main` as `4039df61`) and the pin
advance to it (`f469cf04`) were pushed after a fresh full gate (all six steps exit 0,
2636 tests passed, 0 failed). The Machine (writer host release, generation
`worker-95d6504d…`) and then the Controller were activated on Hawk; see
[the release record](releases/worker-pin-incarnation-carriage-2026-10-07.md). The
Machine now stamps the lineage and the Controller consumes it. The value is
process-local on the Controller and not exposed by any diagnostic, so no production
observation of it has been made.

## Limits

No production observation of a lineage change retiring a real observation exists. Local
runtimes, older Machines and reader-only builds stay on the process-local fence and are
not labelled differently in diagnostics. Lineage is not a state lease. The Controller
does not persist it; a restart relearns it from the Machine.
