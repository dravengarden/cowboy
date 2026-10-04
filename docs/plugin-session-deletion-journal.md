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

The published owner descendant also validates journal and success-receipt
transaction IDs with the explicit repair command's generated-format rule.
Missing, null, empty and malformed IDs refuse before recovery effects or
history filenames can use them. Compatible recovery keeps the original ID even
when the recovered revision changes. The
[transaction identity candidate](releases/plugin-transaction-identity-2026-10-03.md)
passed its gates and clean Hawk build, but host activation was initially refused
before switching by two unrelated failed OVH observation units. Authorized
successor-revision observations subsequently passed 21/21 samples on each host.
An obsolete JMS probe with removed test inputs was separately archived and
retired with explicit user authorization; no JMS network acceptance is claimed.
Owner source `e516fc8c` then activated successfully on Hawk with passing health
checks. Machine and Controller PIDs stayed unchanged; 16 of 17 worker/keeper
PIDs were retained, with the remaining exited stale worker recycled by the
existing broker on session revive. The host transaction's ordinary failed-unit
refusal policy remains unchanged. Production deletion writing remains disabled.

The owner journal also admits only its eight written phases. Missing, null,
unknown and receipt-only terminal phases refuse before automatic recovery can
rewrite intent or mutate the profile. `recovery-selected` requires the
Controller or Machine lane and a nonempty explicit target/revision, so loss of
a selected target cannot revive the implicit predecessor. Existing valid
commit and rollback replay remain unchanged. The
[journal phase release](releases/plugin-journal-phase-2026-10-03.md) records
three-lane rejection/reopen fixtures and successful Hawk owner activation at
Columbus source `193565d0`; no invalid production phase was seeded.

Journal and success-receipt maintenance authority must agree with the lane:
Machine requires true, while Controller/Web cannot carry Machine authority.
Missing, null or false original Machine authorization refuses before automatic
recovery, active-revision acceptance or explicit repair effects. A later
invocation's flag cannot substitute for the original record, and refusal does
not rewrite authorization. Existing invocation requirements and valid replay
remain unchanged. The
[maintenance authority release](releases/plugin-maintenance-authority-2026-10-04.md)
records ten invalid-record fixtures, compatible production receipt inspection
and successful Hawk owner activation at source `ab151b1e`; no invalid production
authorization was seeded.

Published owner source `f5096b38` also requires clean direct children of
`/nix/store` for journal candidate/predecessor and receipt release/active/
predecessor paths, sharing explicit recovery's path predicate. Prefix-only
subdirectories, traversal and redundant separators refuse without normalization;
empty optional predecessors keep bootstrap semantics. Existing manifest,
lane and reader checks remain necessary. The
[canonical store-root release](releases/plugin-store-root-2026-10-04.md)
passed 32 path fixtures, its full gates and clean Hawk build. After explicit user
instruction to go live, two terminated Stormbird tasks were archived and retired,
preserving the paused wave and making no Stormbird rollout acceptance claim.
Owner source `f5096b38` activated successfully on Hawk with passing health checks.
Machine, Controller and all 17 worker/keeper PIDs were retained in the bounded
samples; the host gate's ordinary failed-unit policy remains unchanged.

Ordinary rollback now opens each nonempty predecessor with `ValidateSource`
and requires the journal's lane before restoring a profile or restarting a
service. Unavailable, incomplete and wrong-lane predecessors refuse at target
selection. Machine bootstrap remains a valid ordinary predecessor; explicit
recovery retains candidate-only admission and never falls back. Interrupted
recovery can persist its rolling-back intent before this refusal, so it does
not preserve every structurally valid journal byte. The
[rollback predecessor release](releases/plugin-rollback-predecessor-2026-10-04.md)
records three-lane fixtures and successful Hawk owner activation at source
`8ad13670`, with the 17 worker/keeper PIDs retained in bounded samples.

Older host activators and older portable installers/launchers remain outside
this finite guard. Updated portable paths refuse committed terminal state
outright; compatible portable recovery readers remain unadmitted. A finite
independently supplied old/new declared-reader release pair
and disposable test-executable process crash/reopen/failure fixtures now pass.
Writer-release acceptance and the remaining recovery authorities are still
open; production writing remains disabled. Release metadata is a build-owned
claim, not authorization.

The [activation authority audit](plugin-activation-authority.md) identifies the
actual unrestricted-root and same-user write bypasses. The user selected keeping
sudo rights: administrators stay trusted, and supported deployment is constrained
to the installed owner. Independent old root-capable tools and same-user portable
installers are outside that guarantee; strong actor isolation is not selected.

The [installed-owner release](releases/plugin-installed-owner-2026-10-04.md)
now pins supported dispatch to the installed immutable owner and retires the
caller-built `candidate` transaction option before fetch/build/dispatch.
Controller failed-rollback repair defaults to `installed`; owner changes use
host releases. Hawk source `e4a2b363` activated successfully with unchanged
sudoers hashes, Machine/Controller PIDs and all 16 observed worker/keeper PIDs.
This selected administrator boundary does not fence independent old root tools,
and it opens no production writer or portable compatible-reader admission.

Portable `DesiredComponent` now carries an optional closed
`session_deletion_journal` claim, restricted to the singleton Machine host,
schema-1 reader and disabled writer. Declared readers sign a distinct
`cowboy-component-v4` transcript including the compact canonical declaration;
absent/null claims preserve exact legacy v3 bytes. Adding, changing or stripping
the claim cannot reuse a signature across those domains. Controller manifest
acceptance and Machine preflight reject invalid declarations. This is signed
metadata, not a portable floor or recovery authorization: even a valid declared
reader still refuses committed portable state, and the production writer stays
off. The publication transcript is specified in
[Machine operations](machine-operations.md#component-publication).

The [signed reader-claim release](releases/plugin-portable-reader-claim-2026-10-04.md)
records complete gates, preserved v3 transcript/signature mutation fixtures and
successful separate Controller/Machine activation at source `406471a2`. All
16 observed worker/keeper PIDs were retained; only each owning daemon restarted
in its own stage. A first stale candidate refused before dispatch and was rebuilt
from fresh main. No new signed production component record or portable admission
was published, and deletion writing remains disabled.

## Portable refusal gate

Portable component declarations now bind a Session deletion reader schema,
but persistent reader/recovery admission is not established. A Machine-host reconcile
refuses any committed deletion entry before fetching a payload, before probing
it and before publishing active/rollback/command links. Other component kinds
retain their existing admission. A probe-created record refuses publication;
verified staging and probe effects are not rolled back.

Reconcile also checks staged Machine host bytes against the downloaded,
digest-checked and publisher-signed artifact before running a probe and again
before publishing pointers. Raw hosts must remain regular files with identical
bytes. Archive hosts must retain the exact regular-file/directory tree, including
companions and empty directories; changed bytes, extra entries, links, special
entries and ambiguous archive paths refuse. Expected archive contents are parsed
before extraction. Existing cache entries are checked, not silently repaired.
An authenticated probe may leave effects behind, but modified host bytes cannot
publish. This does not authenticate a cached host on a later launcher start,
bind bootstrap recovery, create a persistent portable floor or fence a concurrent
administrator writing the cache. The [staged-host integrity release](releases/plugin-host-cache-integrity-2026-10-04.md)
records that narrower boundary.

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
