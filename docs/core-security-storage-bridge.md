# Core security storage bridge

This is the twelfth slice of the
[spatiotemporal design](plugin-spatiotemporal-design.md). It moves Passkey
persistence behind a typed core port and fixes first-import and ceremony
isolation failures, without changing the deployed database format or host
policy. It is a reader-compatible preparation step, **not** the final
CoreSecurity ownership cutover or completion of P1.

## Core port, retained physical location

`src/core_passkeys.rs` owns credential and ceremony SQL. `PasskeyStorage` is not
a generic `PluginNamespace`: only the checked startup bridge constructs it,
after verifying the existing migration ledger and completing first import. Its
interface cannot execute arbitrary Plugin SQL or dispatch native operations.

All clones of a `Store` share one `PasskeyBinding`, including clones made before
initialization. Binding is write-once. An initialization mutex is acquired
before import, so two competing bindings cannot first copy credentials into
different locations and only then race the final pointer swap. After binding,
every clone uses the same current namespace; none retains a stale optional
pointer to the old core tables.

Host preflight checks both dialects of every selected WebAuthn storage host
against the exact historical `0001` SQL fingerprints owned by core. Runtime
activation checks all selected hosts before running any storage migration, and
the bridge checks the actual applied ledger before import. An exact signed
Plugin selection alone cannot add, remove or modify a Passkey migration.
Unselected future publications remain readable but do not migrate storage. Other
Plugin storage contracts are unchanged.

The legacy `webauthn` host claim still locates the existing namespace during
startup. It grants no native execution or credential authority. PostgreSQL keeps
the same schema; SQLite keeps the same namespace file. No credential table is
renamed, copied back to the old core tables, or exposed to Machine uninstall.

## Atomic first import and ambiguous old state

For a namespace without the existing `core_passkeys` import receipt:

1. Acquire its write transaction before checking the receipt: PostgreSQL locks
   the import/credential/ceremony tables; SQLite reserves its writer before any
   receipt read, without modifying rows.
2. Require all three destination data tables to be empty. Any nonempty table
   without a receipt is ambiguous, including a partial import by an older
   Controller. Refuse automatic reconciliation; do not overwrite, deduplicate,
   clear rows, or manufacture a success receipt.
3. Import the stopped legacy core source. Preserve credential IDs, owners,
   public-key/counter JSON bytes, names, timestamps, and every ceremony exactly.
   Import is not garbage collection and does not depend on ceremony ordering.
4. Commit all user/admin credentials, ceremonies, and the original import
   receipt together. A database failure rolls the transaction back; a retry
   rechecks the receipt under the same lock.

When the receipt already exists, the old core snapshot is never re-imported.
Current credential updates and deletions therefore survive restart without being
reverted or resurrected from stale legacy rows.

This is startup-only, before requests are accepted. The locks serialize current
initializers; they do **not** establish a distributed single-writer fence
against an independently running older Controller writing the legacy source.
Online ownership migration requires a separate fenced handoff. Ambiguous
historical partial imports need explicitly scoped reconciliation, not marker
deletion.

## Independent ceremony lifetimes

Normal ceremony upsert now purges only records expired at the current time, not
those expiring before the _new ceremony's future deadline_. Starting or updating
one flow no longer removes another still-live flow. Purge and upsert share one
transaction on both backends, so a failed insert cannot commit GC. Updating a
transaction preserves its original creation timestamp.

## Verification and release

The SQLite and isolated PostgreSQL contracts cover exact schema/checksum
rejection, failure at the final import receipt, full rollback and concurrent
retry, byte-preserving import of both credential classes and multiple
ceremonies, refusal of each kind of unreceipted data, competing bindings, clones
created before binding, and restart after current updates/deletions. Ceremony
fixtures cover overlapping deadlines, exact expiry, failed-insert GC rollback
and creation-time preservation. Catalog fixtures prove selected incompatible SQL
fails pure preflight while unselected publication leaves current storage intact.

This is a Controller-only release. All historical SQLx and Plugin SQL bytes,
database ledgers/formats, Catalog artifacts, production selections, native ABI,
Web assets, Machine and worker generations remain unchanged. The existing
Controller reader can still use the same tables and import receipt; this slice
does not introduce a one-way ownership marker or raise a durable reader floor.
These deterministic fixtures are not a real-device login or native acceptance
receipt, and a code rollback does not undo user actions performed since deploy.

## Remaining ownership cutover

Core-only startup without local Authentication packages still requires a durable
namespace/ownership binding, accepted cold and recovery readers, explicit host
policy migration, and supported-client authentication acceptance. Retire local
Password/Passkey package pins and authority markers only after that handoff.
Keep the SDK/native ABI migration and any later physical table move separate;
neither is authorized by this reader-compatible Controller release.
