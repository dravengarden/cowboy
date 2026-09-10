# Plugin installation incarnations

Status: sixth spatiotemporal slice, 2026-09-10. Protocol 11 adds an independent
installation revision to inventory, the approved uninstall intent and its
durable Machine step. This is core lifecycle state, not a Plugin or a new
installer. The reader bridge is accepted for both components and Hawk's cold
bootstrap. This descendant enables schema-two Service admission and installation
adoption on explicitly admitted Machines; it does not enable compensation.

## Identity and compare-and-swap

An artifact digest identifies immutable release bytes. Installing those bytes
twice creates two different installations. `InstallationRevision` is a closed,
opaque type containing 256 bits from the OS random source; it is not a digest,
an ordering clock, an authorization token or a Service auth generation.

The existing Machine operation journal owns a private `installations-v1/`
directory. Each Plugin slot retains its latest revision, predecessor revision,
target digest or removal tombstone, finite effect kind and completion status.
A removal also records the complete uninstall request digest. An explicit
same-release reinstall advances the revision. Uninstall retains a new tombstone
instead of deleting installation authority. Reinstalling after removal advances
again; it never restores an older revision.

The Service preview captures the observed revision. Confirmation rechecks it
alongside the actor, exact release, Catalog trust and affected sessions. A
schema-two Service intent includes that revision in its complete intent hash;
the derived schema-two Machine step includes it in its request hash. The Machine
independently compares it under its lifecycle lock and re-verifies signed release
bytes. A stale preflight stops admission before worker cancellation; a race
after preflight is checked again immediately before the effect.

Reusing an operation identity with changed fields, including a new revision,
is an identity conflict. Repeating an identical completed step returns its old
receipt without removing a later installation. A receipt proves the historical
attempt, not the slot's current state. Controller host-binding snapshots also
invalidate on an observed revision change. This does **not** yet add an
incarnation precondition to every Machine host-invocation command.

## Persistence, uncertainty and rollback

The parent operation journal retains its single-process owner lock. The store's
lifecycle lock serializes installation, removal and activation. Artifact staging
and probes precede the activation intent. Before changing the active link or
credential projection, a tracked install writes `unknown` with a fresh revision.
After successful activation and directory flushes, it persists `stable`.
Uninstall similarly writes its slot intent before removal and its tombstone
before acknowledging the existing durable step receipt.

These two journal records are **not** claimed to be a cross-file transaction.
Failure between them leaves an unresolved slot or step. Either fence prevents
new conflicting lifecycle effects; startup does not infer success from a link,
replay an effect or erase evidence. Even a local activation rollback following
an error leaves its pending revision fenced until verified recovery exists.
Existing detached workers and held code-runtime leases are not killed by a scan.
Service auth revocation remains separately authoritative.

Files are atomically renamed after content flush and followed by a directory
flush. The directory is private (0700), its files private (0600). Evidence has a
canonical checksum and a closed schema, at most 4 KiB per slot and 1024 slots.
Full capacity preserves existing slots and tombstones but rejects new ones.
Corrupt, oversized, unknown-schema or symlinked evidence fails closed. A write
failure poisons the process's installation authority until a validated reopen.
These guarantees assume local Unix storage honors flushes and administrators do
not manually replace the state directory.

Opening the journal and reading inventory never create installation authority.
At an explicitly enabled Machine startup, previously untracked active releases
are verified and assigned initial revisions. Existing records, including pending
ones, are never reconstructed from current links. A reader-only binary keeps
queries and fences but refuses mutations after this namespace exists, including
normal installs and legacy reactivation/removal. It does not downgrade to
digest-only operations or remove the directory to become writable.

## Reader-first rollout

1. Build and accept the reader bridge as both the Controller and Machine
   component, with both new writer constants disabled. Verify the separate
   component receipts, negotiated protocol and worker continuity.
2. Refresh and accept the host's cold-start bootstrap to those compatible
   readers, without moving existing component profiles or writing installation
   authority. A lost profile must not initialize a pre-incarnation reader.
3. Only after those readers and cold recovery are the accepted rollback floor
   may a descendant enable installation adoption and schema-two Service intent
   admission. Activate Machine first, then Controller; the intermediate
   Controller reader pauses new tracked intents. Machine activation is a separate
   maintenance boundary, not a Controller side effect.
4. Reopen retained schema-one and schema-two fixtures with the bridge. Both
   remain queryable; schema-two writes and tracked mutations stay paused.
5. Never roll back to pre-incarnation readers after adoption. Never change
   applied migration checksums, delete journals, lower auth generations or
   mutate production Plugins as a release smoke test.

Protocol 10 remains the minimum for retained schema-one queries. Schema-two
commands require protocol 11. Before adoption, the bridge preserves the existing
schema-one workflow. After adoption, new schema-one requests are not admitted,
even from a protocol-ten Controller. Old exact receipts remain readable.
Schema-one canonical JSON omits the new optional field, retaining its old hash.
Writer startup additionally requires the existing `--plugin-operation-admission`
and its pinned `--service-id`; a newly installed, unadmitted Machine does not
silently adopt state. Removing admission after adoption keeps the reader and
fences, not an untracked legacy mutation fallback.
No SQL migration, Plugin SDK version, Catalog release, native bridge or public
Plugin package changes in this slice.

### Accepted Hawk recovery floor

Reader commit `6a420ff5888c1d983de12f6af539a2fb7405d8e7` was accepted as Controller
at `2026-09-09T23:57:15Z` and Machine at `2026-09-09T23:58:59Z`, with the new
writers disabled. The 15 pre-maintenance worker PIDs and generation
`worker-69e13060b36e4b6b698c` were retained.

Columbus commit `ed3ce1af918403c37b556f5e17ca09791625bae6` pinned both cold
bootstrap readers to that same clean Cowboy revision. Its host transaction was
accepted at `2026-09-10T00:16:33Z`; Controller, Machine, both DeepSeek gateways
and all 15 worker PIDs remained unchanged. Installation state was still absent
at acceptance. The host checks validate both immutable reader manifests, not
just flags an older binary also supports. Falcon's pin and admission are
unchanged. Subsequent writers use Cowboy component transactions, not another
host switch. The separate component receipts are the live activation evidence.

## Agent fingerprint correction

The signed Agent lifecycle tests also found an earlier inventory bug: Agent
inventory advertised its inner Provider fingerprint where generic Plugin
uninstall expected the outer Plugin fingerprint. New Machine inventory now
reports the signed outer fingerprint. The Controller scheduling reader accepts
either the exact trusted outer fingerprint or, for an untracked legacy
inventory only, the exact trusted inner Provider fingerprint. It never compares
fingerprints from unrelated releases or treats the two contracts as identical.
Retained, signed legacy-only Provider Catalog generations have no outer Plugin
contract: their exact Provider contract stays schedulable after slot adoption.

## Verification and remaining work

The subsequent [recovery assessment](plugin-recovery-assessment.md) exposes a
coherent read-only Machine receipt/tombstone snapshot to the Service. It does
not mint a restoration grant or resolve uncertain evidence by observation.

Hermetic tests cover signed same-release reinstall, stale preflight and execution,
changed-request identity conflicts, tombstone provenance, duplicate receipts
after reinstall, pending reopen, reader-only rollback, unadopted slots, capacity,
before/after-effect storage failures, corrupted and symlinked evidence, wire
version negotiation and both Service storage backends.

Durable compensation is still deliberately disabled. It needs a separate typed
recovery step that checks the exact tombstone, fresh policy/auth authority and
bounded leases, and records installation restoration separately from verified
worker/session restoration. No recovery endpoint can currently overwrite a
later install. Cross-restart monotonic lease policy, verified worker restoration,
operator recovery UI, evidence archival, Victoria binding lifecycle and the
generic finite executor remain unfinished. This slice closes uninstall's
same-release ABA gap; it does not claim strict reversibility or the P2/P4 exits.
