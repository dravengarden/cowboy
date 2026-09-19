# Core product permission lifetimes

This finite Controller mechanism adds time identity to the product role policy.
It is not a Plugin, serialized authority, database epoch or general graph grant.
It extends [original Code request authority](plugin-code-read-authority.md) and
the shared product credential continuation used by Operator confirmations.

## Original observation

The API authenticator finishes credential verification by atomically capturing
the effective role and a private core observation for the normalized account.
The verified request carries that observation into its continuation; handlers
cannot replace it with a current account/role tuple after a wait. Sharing or
cloning an observation retains the same lifetime, not renewed permission.
The original user ID, cookie/PAT hash or device identity, expiry, current enabled
user and operation-specific permissions still require independent validation.
No second device proof is consumed.

The settings owner ends affected observations before releasing its mutation
lock. Thus Owner → Viewer → Owner cannot revive a request even if no request
polls during Viewer. Viewer → Operator cannot grant an already queued read new
mutation authority. A fresh authenticated request can observe the new role.
Independent users, unrelated settings and changed policy representation that
preserves the same effective role keep their existing observations.

All existing settings mutation paths share the hook: ordinary set, locked
updates and startup loading. Unwinding a mutation closure reconciles permissions
before unlocking too. One locked mutation is atomic; only its committed effective
roles become visible. Capturing a role and reconciling changes use the same lock
order, with no network/database await under either mutex.

## Bounds and ownership

The Controller bounds outstanding permission lifetimes to 1,024, including
ended lifetimes whose actual holders have not dropped. Concurrent requests for
the same unchanged account share an observation. Weak indexing does not retain
finished requests; capacity is reclaimed only after their last actual holder
drops. Full capacity refuses new observations without evicting an existing one,
renewing an ended one or admitting an effect. Accounts are at most 64 bytes.

Observations belong to the original core instance. A different Hub/Controller
cannot adopt them, even for the same account and role. They contain no credential
secret and have no wire, journal, Debug or serde representation. Controller
restart constructs fresh request observations; it does not restore old grants.

## Consumers and non-goals

Product-backed Code reads and Operator continuations check the original
observation alongside their existing credential checks. Role loss produces the
same closed `401/no-store` response as an ended request credential, rather than
letting an old request inherit a later restored or broader role. A currently
authenticated Viewer still reads its own/shared Sessions; independent Session
visibility refusal remains `404/no-store`.

Native effects are recorded independently of response authority. Ending a
permission observation does not erase an Open/Release outcome, replay a command,
revert a file edit or claim native cleanup. Fresh original-user authentication
may still query and explicitly clean up the original resource by its ID.

The product permission mutation API remains closed in single-user deployments.
Acceptance changes no production role or credential. Source gates exercise
actual Hub policy mutations, queued verified requests, cookie/PAT continuation,
parked successful/conditional/error reads, and owned native-result preservation.
Core gates also cover unrelated-user changes, all mutation paths, unwind,
foreign-core refusal and retained-generation saturation/reclamation. Existing
connected v10 checks remain the actual login/install/Code regression gate; they
do not claim a real HTTP role-mutation test or open an administrative test API.

This covers core-observed product role transitions only. It does not detect
unobserved database disable/re-enable, username/principal replacement or external
policy edits, supply continuous persistent security-domain identity, retrofit
every streaming/legacy handler, or change separate admin-cookie/host-delegation
authority. Current credential revocation checks remain necessary. Atomic
database/network delivery, Machine-owned authorization epochs, state leases,
general DAG execution and independent post-effect recovery remain separate.

The [accepted Controller-only rollout](releases/plugin-product-permission-lifetimes-2026-09-19.md)
records the exact immutable artifact, source negative/positive gates, complete
connected regression chain, browser cases and actual Catalog/host floors.
Its deployment window retains all 16 original workers and unchanged Machine,
Plugin installation/authentication, Web, host and Victoria identities.
