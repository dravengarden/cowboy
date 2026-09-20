# Controller-owned identity for locally executed reads

[Machine-owned Workspace root identity](plugin-machine-workspace-identity.md)
fences reads the Machine executes. It cannot fence the reads the Controller
executes itself, because those never reach a Machine.

This slice states the missing half of the same rule: **the party that touches
the filesystem owns the continuous identity of the root it reads.** It is not
a state lease, a grant, a Plugin capability or a recovery mechanism.

## The hole this closes

Two routes execute on the Controller's own filesystem:

- a Workspace read whose Machine is **colocated** (a local-UDS connection), and
- a Session read whose route is colocated or is a standalone `local` Session.

The primary deployment is exactly this shape: its resident Machine runs in
local mode, so every advertised root it exports and every session worktree it
prepares is read by the Controller directly. `workspace_scope_is_colocated`
and `session_read_is_colocated` return `true` and the reader calls
`LocalCodeProvider` without any Machine command.

Nothing observed the object behind those roots. Deleting and recreating a
root — or replacing it with another worktree or mount at the same absolute
path — left the read observation, its cached representations, its ETags and
its page/diff continuations intact, and the Controller read the new object
under the old identity. The Machine-owned fence never applied, because the
request never left the Controller.

## Ownership

- **Controller** owns and enforces identity for roots it reads itself, and
  only for those. It observes the object when it takes the observation, and
  re-observes before every local execution. It mints nothing for a Machine and
  cannot construct, renew or compare a Machine-owned incarnation.
- **Machine** keeps owning the roots it serves remotely. The two fences never
  substitute for one another: a colocated route is never handed a Machine
  incarnation, and a remote route is never checked against a local stat.
- Identity lives in the observation itself, not in a side registry. A Workspace
  observation records it when the authenticated inventory is accepted; a
  Session read route records it when the route is resolved. There is no new
  map, eviction policy, timer or durable state.

## Why this differs from the Machine's mechanism

The Machine pins each advertised root with a retained directory handle, so the
kernel cannot reuse its inode number and a replacement is necessarily a
different object. The Controller cannot afford that: its soft open-file limit
is 1024 while a single colocated Machine may export up to 1,024 roots, and
those descriptors would compete with its listeners and connections.

The Controller therefore compares device, inode **and creation time**, and
requires the creation time. A recreated directory does reuse its inode
immediately on the deployed filesystem — a source test demonstrates exactly
that — so device and inode alone would pass the fence; the creation time is
what catches it. This *detects* reuse rather than *preventing* it, and it
depends on the filesystem recording a birth time. A root whose creation time
cannot be read is refused for local execution rather than read unfenced.

## What a refusal means

A refused local read performs no filesystem I/O. A Workspace observation whose
root changed is retired exactly as a Machine refusal retires one, so its
caches, ETags and continuations stop answering; the next accepted inventory
observes the live root again. A Session read route carries its identity inside
the route, so a replacement makes the old route unequal to the current one:
in-flight and cached responses become `410`, and the next request resolves a
new route against the new object without an inventory.

Refusal is an ended observation. It is never a rollback, an undo, a repair, or
a claim about a read that already started.

## Out of scope

- Reads already dispatched or already executing. Nothing is cancelled.
- Session worktree *preparation* and its Machine-side identity; this fences
  what the Controller reads, not how the Machine creates a worktree.
- Files and directories inside a root. Only the root object is observed.
- Continuous Machine-owned Session or security-domain identity, state
  reader/writer leases, general graph contracts, independently authorized
  post-effect restoration, native-generation replacement and supported-device
  acceptance all remain open in the
  [completion ledger](plugin-refactor-completion.md).

## Evidence, and the coverage this does not have

The [accepted Controller rollout](releases/plugin-local-root-identity-2026-09-21.md)
records the complete quality gate, four source regressions verified to fail
against the previous behaviour, and the unchanged connected v12 chain.

The connected gate does **not** cover the colocated branch, and that gap is
structural rather than an omission. Its Machine reaches the Controller over
TCP, and `colocated` is derived from the Machine's self-declared
`connection_mode`. A fixture could therefore only become colocated by
declaring local mode over a TCP transport — which is exactly the trust gap
recorded in the completion ledger, not a property worth building acceptance
on. Honest connected coverage of this branch requires binding local execution
to the transport (or to enrollment) first; until then the colocated branch has
source-test evidence only, and the remote branch is covered end-to-end by the
unchanged connected chain.

Source tests use real directories, including the delete/recreate sequence that
provably reuses an inode on the deployed filesystem. Before activation, all 32
roots the primary deployment advertises were observed: 30 resolve to
directories with a creation time and are unaffected; the two that refuse
(`deepseek-harness-cloudflare`, `lasso`) are already-absent directories whose
reads fail today, so only their refusal reason changes.
