# Reader-first Machine Session terminal-deletion journal

The resident Machine opens `state_dir/session-deletions` before accepting any
runtime peer. A closed schema-1 record binds terminal Session IDs to the
configured Machine ID and optional Service ID. Owner mismatch, malformed data,
unknown schema/fields, duplicate IDs and oversized records refuse startup;
only an absent committed file means an empty namespace. Serialized identity
is storage identity, not authorization or a Machine security-domain incarnation.

A loaded terminal ID enters the broker cancellation set before Welcome.
Its surviving worker cannot reconnect, an old `EnsureSession` cannot adopt it,
and reset cannot clear it. This is a permanent slot fence; it does not yet
supply a fresh same-ID Session incarnation or worktree ownership.

## Source writer candidate

The private writer is tested through the real core IPC path. It commits a
sorted snapshot using a new staging file, file sync, atomic rename and directory
sync before deletion side effects or positive acknowledgement. Namespace
creation also syncs its parent. Storage failure leaves the existing worker and
registry untouched, returns a negative acknowledgement and fences further
journal writes/admission in that owner. A rename followed by failed sync is an
unconfirmed partial outcome, not a rollback or a confirmed deletion.

The namespace holds one exclusive file lock and retained directory/lock handles.
Replacement ends admission. Cancelled broker serving drops its monitor task so
an orphaned monitor cannot retain the namespace lock. Uncommitted staging files
are retained and never replayed. The namespace accepts at most 4,096 terminal
IDs, 4 MiB of committed JSON and 512 bytes per ID; exhaustion refuses new writes
rather than evicting prior deletion decisions.

## Production admission remains read-only

The resident production constructor hard-codes the writer off. No request,
CLI flag or environment variable can enable it. A healthy empty reader retains
the existing process-local deletion behavior and writes no terminal record;
this release therefore does not make newly deleted production Sessions durable.
Historical volatile deletes are not reconstructed or migrated automatically.

Older Machine artifacts ignore this namespace. Before enabling the writer,
the component owner must enforce a compatible reader on activation, fallback
and independently authorized recovery, then accept exact old/new readers and
real-process crash/reopen/failure cases. A startup refusal by the new reader
alone does not fence rollback to an older component. This separate admission
is still open; the private candidate must not be enabled by simply changing
the constructor boolean.

## Evidence boundary

Source fixtures close and reopen the journal, and restart in-process broker
owners over real Unix IPC. They confirm pre-Welcome worker refusal, old launch
refusal, terminal reset refusal, failure without Stop or registry effects,
read-only compatibility, owner mismatch, bounded input and exclusive ownership.
They are not an independently supplied old/new executable matrix, an OS-process
crash or power-loss simulation, supported-device acceptance or native resume.
Continuous Session incarnation, worktree ownership and general state leases
remain separate from this finite terminal-deletion dataset.

The [Hawk reader release](releases/plugin-session-deletion-reader-2026-10-03.md)
records exact source/build gates, writer-disabled observation and bounded
production continuity separately from writer admission.
