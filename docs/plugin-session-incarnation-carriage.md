# Carrying the Session lineage to the Controller — implemented, held off main

Fourth step of the [durable incarnation design](plugin-session-incarnation-design.md),
after the [reader](plugin-session-incarnation-reader.md) and the
[writer](plugin-session-incarnation-writer.md), which is active on Hawk. The change
is written, tested and **deliberately not on `main`**: it is the local branch
`incarnation-carriage-20261006` (head `58a26dc7`, two commits on top of `477768bd`) in
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
authorization) gaining a lineage variant. At the first commit the full gate passed
(619 standalone, 1960 all-features) except Clippy; after the simplification Clippy and
the targeted tests passed. The full test suites were **not re-run on the final
commit** of that branch.

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

## Limits

No production observation of a lineage change retiring a real observation exists. Local
runtimes, older Machines and reader-only builds stay on the process-local fence and are
not labelled differently in diagnostics. Lineage is not a state lease. The Controller
does not persist it; a restart relearns it from the Machine.
