# CoreSecurity namespace ownership

The thirteenth [spatiotemporal slice](plugin-spatiotemporal-design.md) adds an
opt-in Controller startup path with **no local Authentication Plugin**. Password,
setup, device authorization, Passkey storage and core Web UI no longer require a
local Catalog release. External OIDC still needs its configured exact signed
driver and renderer. This is not completion of P1 or a native ABI change.

## Explicit core policy

`--core-security-config` / `COWBOY_CORE_SECURITY_CONFIG` accepts an absolute,
private regular JSON file, no symlink, at most 8 KiB. Unknown fields/variants and
invalid identities fail closed; private JSON errors never echo values.

```json
{
  "schema": "dravengarden.cowboy.core-security/v1",
  "passkeys": {
    "namespace_id": "passkey",
    "source": "adopt_legacy"
  }
}
```

This selects core ownership of a stable location, not a Plugin to resolve.
PostgreSQL keeps `plugin_passkey`; SQLite keeps
`plugins/live/passkey/state/db.sqlite`. No table is renamed or moved. The portable
namespace identity is a closed lowercase slug, at most 56 bytes. `source` is
required and becomes immutable once prepare commits:

- `fresh` accepts an empty security source with no prior Plugin storage
  namespace/authority. It atomically creates the core baseline and an empty
  import receipt. Choosing a new name cannot bypass existing credentials.
- `adopt_legacy` opens an existing namespace only, requiring the exact historical
  `0001` ledger and an already completed `core_passkeys` import receipt from the
  [storage bridge](core-security-storage-bridge.md). It never imports stale core
  rows, runs a package migration or repairs ambiguous data.

Durable storage is mandatory; in-memory SQLite is rejected. Core mode requires
`catalog_only`; absent host configuration defaults to that source in this mode
only. Supplied local-auth pins are errors, not silently ignored. Unselected
historical local packages remain readable, without a default or migrated
namespace. The core namespace is reserved against Plugin selection/migration.

Existing authentication policy remains authoritative: password/Passkey toggles,
setup, sessions, capacity, logout, account mappings and secrets do not move into
this file. An external-only configuration gains no password fallback. Exact
OIDC selections still merge into host policy and must remain available.

`serve --check-plugin-hosts` shares this preflight, including the observed
filesystem authority, but remains **configuration-only**. It adds bounded core
namespace/source metadata, not database paths, credentials or ownership tokens.
It does not connect to the database or check its authority/import/token, migrate,
stage, execute Plugins or authenticate. Success is not an activation receipt.

## Durable handoff and recovery

Every new Controller, including legacy mode, holds a private lifetime lock under
its data root. The owning deployment must stop the old Controller before
handoff. This fences cooperating local readers using that root; it is not a
distributed lease or protection against an independently launched older binary.
Two Controllers using the same database through different roots are not a
supported online migration strategy.

Security initialization follows core SQLx migration and precedes session
restore, background writers and Plugin activation:

1. Validate the source, then claim singleton `core_security_authority` with
   `prepared`, an immutable random binding ID, a closed backend/location record
   and namespace/source identity. A competing claim must match the winner.
2. Publish that identity as `plugins/.core-security-v1` using synced, create-only
   complete bytes. Never overwrite a different/corrupt marker. A crash before
   publication resumes from the committed database row.
3. Initialize or adopt the namespace and commit its matching
   `_cowboy_core_security_owner` token. Fresh schema, original migration ledger,
   empty import receipt and token share one transaction. Adoption adds only core
   metadata, retaining the old ledger and current credential/ceremony bytes.
   PostgreSQL serializes initialization; SQLite reserves its writer before reads
   and uses FULL WAL durability. New parent directory entries are synced.
4. CAS the core row to `ready`, then attach the write-once typed Passkey port
   shared by every Store clone. An interrupted prepare checks the same identity
   and token; it never copies rows or chooses another authority.

The two SQLite files are not one atomic transaction. Prepared/ready makes that
explicit. Errors retain evidence and stop startup, without destructive automatic
compensation. A ready namespace is **open-existing only** and must have its exact
ledger, import receipt and token. Lost/replaced storage, changed location,
mismatched core database, missing configuration or attempted legacy fallback
fails closed. The DB guard protects the prepare-before-marker window; no core
namespace has been initialized in that window yet.

The filesystem marker requires core configuration even if the DB flag is
omitted. Generic Plugin storage checks its core Store authority and the
filesystem reservation before migration, including interrupted prepare. Local
security is not part of Machine Plugin uninstall.

Namespace paths and SQLite sidecars reject links/non-regular files at startup.
The owning filesystem/OS account remains trusted; this is not a sandbox against
a concurrently malicious root/Service user. Core and namespace backups must be
coordinated. General PostgreSQL-to-SQLite store-copy refuses a core-owned source:
changing backend/location requires a separate namespace/authority migration.

## Reader floor and production activation

This release adds **empty additive** PostgreSQL `0046` / SQLite `0020` migrations.
Historical SQLx and Plugin SQL bytes remain immutable. Without core configuration
it creates no ownership row/token or core marker, retaining existing local-auth
pins. The predecessor's additive-migration reader still supports that state.

Production activation for this slice is **reader-only**, under existing
generated policy. Do not remove live Password/Passkey pins, edit generated JSON,
delete markers or use a component restart to perform a host-policy cutover.

Later cutover requires a successful new-reader receipt and actual
active/automatic-recovery reader floor, accepted client authentication, the
owning committed host-policy change and a stopped-Controller handoff. Once
prepare exists, recovery must retain matching core configuration and readers.
A descendant code revert must preserve this format; deleting authority to run
older code is not rollback. Historical package and renderer/native ABI
retirement remain separate until those acceptances are complete.

## Evidence and limits

SQLite and isolated PostgreSQL fixtures exercise creation, adoption, credential
updates/deletions, ceremony bytes, cold reopen, interruptions after prepare and
marker, before/after namespace commit, a database-rejected ready commit, Plugin reservation,
source/location conflicts and refusal to recreate lost ready storage. Policy
fixtures exercise private/closed parsing, no-write empty-Catalog preflight, local
pin rejection and exact external-only OIDC policy. Core baseline SQL is checked
byte-for-byte against the immutable historical package fingerprints.

A loopback HTTP fixture exercises the real auth router after the shared core
startup path and empty host activation: setup, registration, password login,
Passkey registration options/listing, device authorization start and browser
session survival after Store reopen. This is not real-device WebAuthn/OIDC,
device-authorization completion, iOS origin/gesture or native ABI acceptance.
The gates use no live Service/Provider credentials.
