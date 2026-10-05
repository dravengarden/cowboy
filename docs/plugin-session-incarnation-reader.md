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

## Limits

This is validation and exclusive ownership of an empty-by-default namespace. It
does not mint identities, rotate on reset, fence stale observations, carry a
value to the Controller or add a state lease; those are later steps. Portable
launchers do not read the namespace. Falcon's owner has not learned the
declaration, so a release declaring it must not be activated there yet.
