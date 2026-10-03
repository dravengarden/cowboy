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

Machine release metadata now declares `sessionDeletionJournal` with
`readerSchema: 1` and `writerSchema: 0`. The Columbus component owner rejects
nonzero writer declarations and requires a declared schema-1 reader whenever
a committed deletion entry exists. Its checks cover dispatch, locked candidate
and fallback admission, profile restoration and interrupted recovery, including
already-healthy predecessor and Git-pinned candidate paths. Corrupt or dangling
entries cannot be treated as absent state. Empty reader namespaces remain
compatible with a legacy fallback; pending files are not committed evidence.

The component owner now persists a root-owned reader floor after retaining an
already accepted compatible fallback. It syncs that floor before profile
mutation. The floor binds the Machine, dataset path and exact reader anchor;
an empty or missing user-owned dataset no longer admits a legacy reader.
Malformed or mismatched owner state fails closed. The first reader-only
transition from a legacy artifact remains possible; the next activation
anchors that accepted reader. No historical deletion records are reconstructed.

Explicit Machine recovery is limited to a declared compatible reader in the
candidate's exact worker generation. The independently accepted target must
pass remote/active/candidate ancestry, is retained through interruption, and
is revalidated before restoration. A missing or changed target cannot select
the legacy predecessor. Its actual revision and generation enter the recovery
receipt. Cross-generation recovery remains unadmitted for Machine.

Machine maintenance can now select a new compatible target for one exact failed
transaction through `--recover-transaction`. The journal must still await
rollback and retain maintenance authorization; a completed transaction cannot
reuse that approval. The new target must declare the reader, keep the failed
candidate's exact worker generation, pass the owner floor/dataset checks and
integrate fresh main, active provenance, the failed candidate and any previously
selected recovery revision. Selection retains an independent GC root and
archives the original decision before replacing the journal. It changes no
success receipt before the existing rollback, health and pinning engine runs.
Unavailable or changed selected targets never revive the predecessor. The
[same-generation repair release](releases/plugin-machine-repair-2026-10-03.md)
records owner tests and activation; no production failure was seeded.

The owner now bounds journal, success-receipt and existing recovery-selection
archive reads to 64 KiB each; its floor remains limited to 8 KiB. It opens only
regular files without following final symlinks or blocking on FIFO admission,
and limits bytes read after file metadata inspection. Journal/receipt JSON and
reader declarations/floor require exact field names and reject duplicate or
unknown keys, case aliases and trailing values. Existing canonical schema-1
records remain readable. Refusal leaves owner evidence unchanged. The
[bounded owner-state release](releases/plugin-owner-state-2026-10-03.md)
records the parser/control fixtures and host activation. This does not add
cross-generation or writer admission.

Older host activators and older portable installers/launchers remain outside
this finite guard. Updated portable paths refuse committed terminal state
outright; compatible portable recovery readers remain unadmitted. A finite
independently supplied old/new declared-reader release pair
and disposable test-executable process crash/reopen/failure fixtures now pass.
Writer-release acceptance and the remaining recovery authorities are still
open; production writing remains disabled. Release metadata is a build-owned
claim, not authorization.

## Portable refusal gate

Portable component declarations do not bind a Session deletion reader schema.
Until that reader/recovery authority is supplied, a Machine-host reconcile
refuses any committed deletion entry before fetching a payload, before probing
it and before publishing active/rollback/command links. Other component kinds
retain their existing admission. A probe-created record refuses publication;
verified staging and probe effects are not rolled back.

Installation and refresh check the same gate before changing bootstrap payloads,
identity or launcher configuration. Newly generated launchers run the
installer-owned bootstrap's `--check-portable-session-deletion` diagnostic
before selecting active or bootstrap hosts. The diagnostic only inspects
namespace entries; it opens no Machine stores or Controller connection. A
bootstrap without this diagnostic fails closed. A healthy empty or staging-only
namespace retains existing behavior. Committed files, directories and dangling
symlinks all refuse; invalid namespace entries and inspection errors do not
become empty state.

Before replacing a bootstrap, the current installer probes the caller-selected
executable with a cleared environment and temporary HOME/XDG/state. Empty state
must report the exact read-only guard result without creating state; a synthetic
committed entry must return the specific refusal without changing that entry
or opening stores. Each probe has a five-second deadline and bounded parsed
output. A pending probe is killed as its own process group and reaped on exit.
Install/refresh check before copying payloads or changing configuration;
register also checks before binding local origin or creating identity. This
prevents a new guarded launcher from being paired silently with an old bootstrap.
It is a compatibility check of caller-selected installation code, not a sandbox
or signed reader/recovery admission.

The current installer now captures all three bootstrap payloads into an owned
mode-0700 temporary bundle before either probe. Both diagnostics execute that
copy, and install/refresh publish from it without reopening caller paths.
Register retains the same captured bundle across local identity creation and
installation. Replacing original paths during a probe cannot substitute the
installed host or companions. Success and refusal drop the owned temporary
bundle. Capture is sequential, not an atomic upstream bundle-version snapshot;
trusted candidate code and its external dependencies are not sandboxed or made
immutable. The [bundle snapshot release](releases/plugin-bootstrap-snapshot-2026-10-03.md)
records the exact packaged installer acceptance.

Rejected Welcome reconciliation no longer requests a host restart. This keeps
a refused candidate from repeatedly exiting its healthy resident reader.
These are bounded refusal checks, not a portable reader declaration, persistent
floor, signed recovery selection, atomic writer transaction or power-loss
acceptance. Older installers can still replace their own launcher; this slice
does not fence that independent authority. The production writer remains off.

## Evidence boundary

Source fixtures close and reopen the journal, and restart in-process broker
owners over real Unix IPC. They confirm pre-Welcome worker refusal, old launch
refusal, terminal reset refusal, failure without Stop or registry effects,
read-only compatibility, owner mismatch, bounded input and exclusive ownership.
Additional fixtures launch disposable broker OS processes from the freshly
compiled test executable. They SIGKILL at staging, file-sync, rename and
directory-sync boundaries, then reopen from a fresh reader process over real
Unix IPC. Pre-rename staging is never replayed; published terminal records
fence workers before Welcome and reject both ordinary and adoption-only
launches. An acknowledged deletion survives writer and subsequent reader
SIGKILL. Foreign logical owners and invalid committed storage refuse startup;
a storage failure refuses the ACK and further admission in its existing owner.
Every child is killed or exits and is reaped. All checkpoint hooks and child
entry points are test-only.

These fixtures are not an independently supplied old/new release-executable
matrix, power-loss simulation, supported-device acceptance or native resume.
Visibility after rename and SIGKILL is not proof of power-loss durability or
a successful deletion ACK.

A separate opt-in integration fixture starts two independently supplied
immutable Nix Machine releases with different source revisions and native ELF
digests. The declared schema-1 readers preserve a synthetic committed record
through old/new/old SIGKILL and reopen. Both refuse malformed, incompatible,
foreign-owner, oversized-by-record-count, nonregular and symlinked committed
inputs before binding the broker. Empty and staging-only namespaces retain
volatile deletion behavior without writing or replaying records. This is one
exact Linux reader pair, not an undeclared legacy-reader admission, a host
activation transaction or a production writer-release acceptance.

Continuous Session incarnation, worktree ownership and general state leases
remain separate from this finite terminal-deletion dataset.

The [Hawk reader release](releases/plugin-session-deletion-reader-2026-10-03.md)
records exact source/build gates, writer-disabled observation and bounded
production continuity separately from writer admission.

The [component compatibility release](releases/plugin-session-deletion-compatibility-2026-10-03.md)
records the installed Hawk owner guard, declared reader envelope and live
generation/continuity observations separately from the remaining writer gate.

The [reader-floor release](releases/plugin-session-deletion-floor-2026-10-03.md)
records the root-owned persistent floor, actual undeclared-artifact refusal and
bounded same-generation recovery fixtures. Older recovery authorities and
writer acceptance remain separate.

The [process-crash acceptance](experiments/plugin-session-deletion-process-2026-10-03.md)
records the disposable process matrix, source gates and remaining release
boundaries. It changes no production writer or deployment admission.

The [immutable reader-pair acceptance](experiments/plugin-session-deletion-releases-2026-10-03.md)
records the exact release revisions, native and launcher digests, isolated
31-process matrix and remaining writer/host recovery boundaries.

The [portable refusal release](releases/plugin-session-deletion-portable-2026-10-03.md)
records the source guard, executable launcher tests and Machine release receipt.

The [bootstrap compatibility release](releases/plugin-bootstrap-guard-2026-10-03.md)
records pre-copy probe checks and exact old/new installer refresh controls.
