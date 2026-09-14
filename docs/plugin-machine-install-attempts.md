# Machine installation attempts

Status: implemented reader and finite executor, 2026-09-14; new Machine writer
admission remains disabled. The [Service schema-two bridge](plugin-install-journal.md)
now binds observed targets and atomically stores exact typed receipts; its fresh
admission is also paused, without a protocol-seven fallback. The bridge has now
[replaced the active Service and Machine readers on Hawk](releases/plugin-install-receipt-readers-2026-09-14.md),
including accepted actual next-recovery and cold roles. This is reader-floor
acceptance, not connected installation execution or completion of P4 recovery.
The current descendant enables its code switch as a writer candidate; the
standing `--plugin-operation-admission` and original connection-bound authority
remain mandatory. Its [connected acceptance gate](plugin-install-connected-conformance.md)
must pass before activation; the running reader remains writer-disabled.

## One core installation path

Protocol 19 adds an exact target observation, an installation step, and a
read-only step query. Agent, code-intelligence and telemetry capabilities use
the existing `MachinePluginStore` installer and lifecycle lock. Communication,
installation, authority, native hosts and the journal remain core mechanisms.
No Plugin executes a workflow engine or owns another installation lifecycle.

`InstallTarget` distinguishes a genuinely vacant slot, an installed incarnation
plus exact artifact digest, and an uninstall tombstone. Empty inventory is not
vacancy. The Machine reads the actual active link and durable slot under its
lifecycle lock; a current installed target must pass its existing signed
generation verifier. It compares the target before staging and again before
activation. Reinstalling identical bytes still produces a new incarnation.

`InstallStep` binds Service, Machine, operation ID, complete Service-plan digest,
exact Plugin release, expected target, complete desired-envelope digest and the
original deadline. All enums and request records reject unknown fields. The
journal keeps no package bytes, artifact URLs, credential values or grants.
Authentication Providers remain non-installable on a Machine.

The original connection creates a non-clonable, non-serializable execution lease
before scheduling or waiting for the lifecycle lock. Its process-monotonic cap
is five minutes, further bounded by the originating absolute deadline. The
Service now computes installation expiry from the original confirmation time;
queueing and validation cannot reset the remote deadline. Neither a new socket,
a refreshed login, a queried receipt nor restart can renew that lease.

## Durable boundaries

| Persisted phase | Next permitted work |
| --- | --- |
| Prepared | Recheck the original lease; no Plugin effect yet |
| Staging | Publisher pin, package/runtime staging and bounded probes |
| Activating | Installation-slot intent, then active-link change |
| ProjectingAuthentication | Agent-only projection of the already authoritative Service replica |
| Applied | Historical completed installation incarnation; no next effect |

Each phase is committed before its effect, then the original lease is checked
again after the flush. Runtime download/probe waits share its remaining budget;
connection loss stops subsequent downloads, probes, activation and projection.
Synchronous filesystem calls are not hard real-time or suspend-inclusive
timeouts. Already started I/O cannot be revoked retroactively.

The durable path flushes staged regular files and directory entries before
activation. It does not follow archive symlinks and bounds the flush traversal.
The existing installation-slot journal records a fresh pending incarnation
before activation/projection and a stable incarnation before the attempt can
acknowledge Applied. These records are not a cross-file transaction. Failure
between them leaves a pending slot or attempt and blocks conflicting lifecycle
effects. A completed historical receipt does not prove current installation or
restored native sessions.

Prepared may become Rejected when its original lease ends before Staging. Once
staging has begun, failure or lost authority is Unknown, even if no active link
changed. An inline activation error may restore its local link snapshot while
the original lease is still valid; this does not clear pending evidence or
constitute durable independently authorized compensation. Credential authority
is never rolled back.

## Storage, duplicates and recovery readers

The parent Machine operation journal retains its single process owner lock.
`install-attempts-v1/` stores at most 4096 receipts, each at most 8 KiB, with a
closed schema and complete-request checksum. Files are 0600, the directory 0700;
atomic replacement is followed by directory flush. Capacity preserves history
and unresolved references instead of silently pruning them.

Only a process-local continuation can advance its exact prior receipt and
phase. Identical IDs return historical evidence without execution; changed
inputs are conflicts. Startup retains pending evidence verbatim. It never
replays, infers success from inventory, repairs a missing slot from links, or
converts an old receipt into a continuation. Missing installation authority or
a required incarnation slot fails startup closed. Corrupt, future-schema,
oversized and symlinked records also fail closed; ambiguous writes poison local
mutation authority until validated reopen.

Attempt presence permanently prevents legacy install/reactivate/remove entry
points from bypassing this namespace. Pending attempts share the existing
uninstall and host-operation fences. Independent Service authentication
authority is not revoked merely by an installation fence.

## Acceptance

Unit/integration fixtures cover complete request identity, changed envelopes,
expired/disconnected original authority, signed installs and same-release ABA,
all pending-phase reopens, before/after-effect storage failures, missing
authority, and original reply-kind/connection correlation. The retained signed
Agent and code fixtures exercise the same durable path, including Agent auth
projection and loss of authority at its final boundary. They do not use
production credentials or installations.

From clean committed source in the pinned Linux shell:

```sh
nix develop -c just plugin-machine-install-reader-conformance /absolute/matrix.json /absolute/new-receipt.json
```

The matrix has schema 1 and a `machine` object naming absolute immutable
`active`, `rollback` and `cold` release outputs. Seventy-two checks run twelve
absent/historical/pending/corrupt/missing-authority cases across all three roles,
opening the same disposable state twice. They negotiate protocol 19 and check
exact queries, changed inputs, foreign ownership and conflicting-slot fences.
The intentionally partial historical codecs do not assert a currently restored
installation. Processes receive no writer admission or fresh installation
commands. An expired exact-identity duplicate must return its old receipt even
with an unverifiable historical envelope, without executing the installer.
Private fixture keys, network isolation, exact ELF/source identity and bounded
receipts follow the existing immutable conformance harness.

Candidate-only checks do not establish actual host roles. Machine activation is
a separate maintenance boundary. Before enabling the writer, accept the
Service coordinator's persisted target/step/receipt binding, both actual reader
floors and explicit writer admission. Then accept connected failure/restart and
lost-response behavior with immutable artifacts. Independently authorized
post-effect restoration, evidence archival, unified recovery diagnostics and supported
native-generation acceptance remain separate exits.

The first Machine-only reader candidate `46dedfa959b5f50ee8d44bf024478c955fd0f6a7`
passed all 72 checks using its immutable release in all three candidate roles.
The private receipt SHA-256 is
`9a6521c7b8b553bda9bc1f131d02fe708a441e819582f05ea739b2b0ac746234`.
It was not activated and does not establish the actual active/recovery/cold floor.
