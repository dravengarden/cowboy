# Session incarnation reader (schema 1, writer disabled)

First implementation step of the [durable incarnation design](plugin-session-incarnation-design.md).
Every Machine build opens `state_dir/session-incarnations` at startup, validates
`incarnations.json` if present and refuses to start on invalid state, exactly as
the [deletion journal](plugin-session-deletion-journal.md) reader does. **No build
writes it**: release provenance declares
`sessionIncarnations {readerSchema: 1, writerSchema: 0}`, nothing mints, rotates
or ends an incarnation, and no Machine behaviour depends on a stored value.

## Record

A closed document `{schema, owner, entries}`; the owner must equal the configured
Machine and Service. Each entry is `{session_id, incarnation, epoch, origin}`:
`incarnation` is exactly 32 lowercase hex digits, `epoch` an unsigned integer and
`origin` one of `minted`, `adopted`, `reset`, `rebound`. Unknown fields,
duplicate Session IDs, an incarnation shared by two slots, another schema or
owner, a nonregular file or link, more than 4,096 entries and more than 2 MiB
refuse. An absent file is empty state; `.pending-*` staging files are never read.
Refusal never rewrites the file.

## Ownership

The namespace is created without following links, holds one exclusive lock file
with retained directory and lock handles, and ends admission if the directory,
its parent alias or the lock is replaced. A second owner refuses ("already
owned"). The reader holds the lock for the life of the resident so a later
writer build cannot overlap it.

## Admission and provenance

The installed Columbus owner learned the declaration first (Hawk, Columbus
`5af9f70b`). Until a root-owned incarnation reader floor exists, an artifact
without the declaration remains valid; this release is the first that declares
it, and the floor is anchored by the activation after it. The native writer
startup parser and the reader-conformance manifest accept the declaration only
as reader 1 / writer 0, and still refuse unknown fields.

## Status: activated on Hawk (2026-10-06)

The reader, the bounded cleanup retries and the shared `namespace.rs` are in the
Machine release `48d3054d` that another task activated on Hawk as transaction
`1791247667920288462-48d3054db95c` (writer host release
`/nix/store/nmr8a6kr…`, worker generation `worker-135348a7…`, pin advanced to
`8909c1c8`). Startup logged `Session incarnation reader ready incarnations=0`,
the deletion journal reader with `deleted_sessions=5 writer_enabled=true`, and
`durable Session cleanup continuations ready pending=0`. No incarnation record
exists, and the owner has **not** yet created the incarnation reader floor: the
previous release did not declare the dataset, so the first reader-only transition
creates none and the next activation anchors `48d3054d`. A writer is still
refused until that floor exists.

The native conformance extension was run on built artifacts afterwards, comparing
the active `48d3054d` writer with the `a0394f67` writer and reader-only
releases of the same worker generation: **37 groups accepted**, including the
five incarnation vectors (default reader owns an empty namespace and refuses an
invalid record before binding; valid record reopened untouched; corrupt,
foreign-owner and shared-lineage records refused before binding). The
[receipt](experiments/incarnation-reader-native-conformance-2026-10-06.json) also
covers the deletion journal after the `namespace.rs` refactor. The activating
task's own acceptance of `48d3054d` is separate and was not reviewed here.

## Earlier status: published, not activated

Source is on main (`4d792318`) after the full Rust gate on the integrated tree
(585 standalone and 1918 all-features tests, both Clippy gates, Rustfmt). The
native conformance extension (default reader owns an empty namespace; a committed
valid record is accepted and untouched by the previous writer; corrupt,
foreign-owner and shared-lineage records refuse before binding) has **not been
run against built artifacts**. Main now contains the independent commit
`22de6bbf` (host resources and idle hibernation), which changes the Machine
runtime wire files. `cowboy-machine-host-release` and
`cowboy-machine-writer-host-release` refuse that change by design
(`retained-worker-interface-compatible`), so no resident-only release can be
built from main. Activating it needs the separate worker-pin/full-release
maintenance acceptance, which is not authorized by this work and was not
attempted. Production still runs `9c79b9c3`; no floor was created.

## Limits

This is validation and exclusive ownership of an empty-by-default namespace. It
does not mint identities, rotate on reset, fence stale observations, carry a
value to the Controller or add a state lease; those are later steps. Portable
launchers do not read the namespace. Falcon's owner has not learned the
declaration, so a release declaring it must not be activated there yet.
