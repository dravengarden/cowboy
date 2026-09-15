# Product browser datasets

This is a finite core state boundary, not an installable Plugin or a serialized
authorization. The generic composition checker still grants no live authority.
The [completion ledger](plugin-refactor-completion.md) retains the remaining
general state, native and independently authorized recovery exits.

## Identity and transport

The authenticated `GET /api/sync/dataset` returns an exact, no-store descriptor:
schema `dravengarden.cowboy.product-sync-dataset/v1`, dataset ID, immutable user
ID, database version 2 and outbox contract `atomic-delta-v1`. The dataset ID is
SHA-256 of the canonical JSON tuple `("cowboy.product-sync.v1", Service ID,
user ID)`. It is stable across reconnects, Controller replacement and account
renames, but not across different Services or users. It is not a secret or grant.

Auth projects `user_id` alongside the display account. The core auth root must
bind that identity before mounting product consumers; a missing ID cannot be
silently substituted with a label. A changed principal tears down the old
product graph synchronously before awaiting cache deletion. Transient auth
outages remain outages, not logout. Auth never imports the product store or
opens a product socket.

The Web owner discovers at most 2 KiB of strict JSON using authenticated,
cache-free HTTP. It freezes one descriptor and does not follow a later cookie,
Service, schema or dataset replacement. Reconnect verifies the original
descriptor; an observed changed dataset permanently fences that owner, even if
the old identity later returns. Only a new product root may adopt a new dataset.

`/ws` validates the dataset against the actual authenticated principal and
Service, then negotiates `cowboy-sync-v1`. Supplied mismatches return 409;
missing required protocol returns 426. The Web rejects readiness and frames
without the selected protocol, so an older Controller that merely ignores an
unknown query cannot admit this client. The existing cookie/Origin/device proof,
capacity class and operation-role checks still apply. Cookie clients cannot
claim the CLI class to evade the browser rule. No Plugin or Machine protocol
changes, login side effects or session rebinding are introduced.

## Storage and lifetime

The core product owner opens `shared-utils-sync`, store `clients`, at exact
version 2. Upgrading creates a missing store but preserves existing stores,
keys and values. Existing connections retire on version change; requests for
the old version fail rather than downgrade. This uses native
[IndexedDB version-change and connection semantics](https://w3c.github.io/IndexedDB/).

Owned keys are `cowboy:dataset:<dataset-id>:service:title`, `:service:order`,
`:session:<session-id>:queue` and `:session:<session-id>:mobile-review`.
Only the closed core scopes are accepted. Web/runtime build generations are
not dataset identities: replacing a component must not lose pending mutations.
Service, user, Session and state remain distinct. This does not add a general
exclusive state-dataset grant or infer arbitrary workspace identity.

The exact `{base,pending}` envelope, mutation identities, atomic delta merge and
load-result handoff from [atomic outboxes](atomic-idb-outboxes.md) remain.
Newly created clients must also hydrate and adopt the exact load result before
writing. Title/order/review changes now use the same durable-before-send
barrier as queues. Failed persistence never grants permission to resend.

State-sync-idb 1.7.0 adds explicit `schemaVersion`, optional
`connectionLifetime: "transaction"`, and strict bounded key inspection. Generic
owners retain their v1/owner-lifetime defaults. The product selects transaction
lifetime: a connection closes only after all its admitted transactions reach a
terminal event. Logical outbox ownership and its delta baseline survive this
idle close; the next operation opens the same exact version. No transaction is
aborted or replayed to implement this policy. Cleanup still reports draining or
failure honestly.

In pinned Firefox 151.0.1, development acceptance reproduced a new-open timeout
after a failed lower-version open and intervening reads while another handle
remained open. A separate native-IDB-only reproduction showed the same result;
closing the idle handle allowed the next open. The product's transaction-scoped
handle policy addresses this observed sequence without extending the 10-second
deadline. This is not a claim about every Firefox version or an identified
upstream bug. Concurrent real Worker writes, abort and restart checks must still
pass with the policy enabled.

## Unowned legacy records

Old `cowboy:sync:*` records contain no trustworthy Service/user attribution.
They remain unchanged and are never automatically loaded into the new dataset,
imported, deleted or sent. Server-acknowledged state reloads from the Service;
old browser-only pending records require explicit human review.

Settings → About → Storage reports older browser records separately from
telemetry. Retention alone is not a warning. Recovery is collapsed by default;
opening it shows one native record selector, previous/next controls and one
local JSON download action, rather than a tall list of download buttons.
Numbered filenames contain no account/session identifiers. Export failure is
reported for the selected record without hiding the inventory or claiming the
original was deleted. Enumeration is strict and capped at 4096 keys; unavailable
storage or overflow is an error, not an empty/complete inventory. Export is
JSON-only, bounded to 8 MiB, 100,000 nodes and depth 64; malformed, cyclic or
oversized values are retained without exporting. Downloads may contain private
prompts/attachments and declare `replay_authorized: false`. They neither
authorize import nor determine whether an old operation already took effect. The
view seals late callbacks on unmount or product-session end and releases its own
URLs/timers. It never closes the shared dataset owner.

`just settings-recovery-browser-conformance <exact-firefox>` exercises the real
React/MUI view at mobile width with 376 synthetic native-IDB records, bounded
layout, read-only export, per-record failures and session-end/unmount cleanup.
It also checks that mobile telemetry diagnostics mount only when requested;
Desktop keeps them visible by default. Journal absence does not indicate whether
separately configured export is running. Collapsing diagnostics disposes the
observer/preview, not a submitted operation. This fixture has no production
account, private backend or physical iPhone download acceptance.
The [About recovery release](releases/about-recovery-2026-09-15.md) records the
Web-only activation and verification.

## Rollout and recovery

Controller/Web order and recovery compatibility are separate from a passing
source gate. The historical Controller bridge supports both missing-dataset old
Web and exact new Web, but always rejects an incorrect supplied dataset. Its
`REQUIRE_BOUND_BROWSER=false` was explicitly **not** completed old-client fencing.
Hawk's dataset-aware Web and compatible cold floor were activated on 2026-09-15.
The [subsequent bound release](releases/dataset-bound-maintenance-2026-09-15.md)
removed that compatibility switch and is active: browser/native-shell clients
without a dataset receive 426, while CLI clients retain their independent
authentication/class boundary. The actual next-transaction recovery and cold
Controller also passed the bound-mode checks. Stale PWAs need a hard reload;
a WebSocket reconnect cannot load new JavaScript.

Retain dataset-aware Web and Controller artifacts for recovery. Do not lower
the IDB version, delete records or reactivate an old blind writer to make a
rollback appear successful. A legacy Controller's refusal of the new client is
safe refusal, not usable dataset compatibility. Accept the actual active,
next-transaction recovery and cold roles separately; an ordinary component
release does not authorize a host policy or resident Machine maintenance change.
The first Web cutover must specify accepted dataset-aware Web recovery through
the machine-owned transaction; restoring only an old bundle cannot undo an IDB
upgrade. Subsequent Web transactions may use their now-compatible predecessor.

Run `just product-sync-controller-conformance <matrix> <new-receipt>` from clean
committed source in the pinned shell. Its closed input contains `readers` (the
existing schema-one Controller active/rollback/cold matrix) and three `modes`
(`legacy`, `bridge`, `bound`) in that order. It launches each exact immutable
Controller twice on the same disposable state, uses real temporary password
login, checks no-store identity and role boundaries, and exercises real browser
and native-shell-class WebSocket handshakes. No production account or actual
native client participates. Use the separate 8-case IDB and 16-case outbox
real-browser gates for storage; neither test substitutes for the other.
Optional `lifecycle_history` flags additionally exercise the
[core durable history](plugin-lifecycle-history.md) against selected actual
readers. They default to false; absence is not history-projection acceptance.

Component registry 3.6.0 appends state-sync-idb 1.7.0 only. All seven Plugin
sources, versions and 2.9.0 pins remain unchanged; no Catalog publication or
Plugin installation is needed. Source publication, actual component activation,
compatible recovery floor and physical-device acceptance must be recorded
separately before describing this migration as complete.
