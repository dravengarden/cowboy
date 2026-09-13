# Connection-scoped runtime bindings

Second implementation slice of the [spatiotemporal design](plugin-spatiotemporal-design.md).
This strengthens existing Core communication and the actual Service → Machine
telemetry path. It does not execute composition proposals or introduce a second
Plugin installer, transport, security authority or runtime dependency.

## Live connection ownership

Core creates an opaque connection token after Machine authentication. Tokens
cannot be decoded from JSON; replacing a channel creates a distinct in-process
identity even if an external epoch string is accidentally reused.

RPC registration, protocol/binding checks, enqueue, inventory observations and
connection replacement share one short lock. No lock crosses an `await`.
Replies must match the authenticated connection **and** expected response kind,
not just a `request_id`. Late/foreign replies cannot consume another waiter.
Old connections cannot replace live inventory or remove a newer channel.

Pending RPCs are limited to 256 per Machine and 4,096 total. Duplicate pending
IDs fail before enqueue. Cancellation, timeout, failed send, disconnect and
replacement release their observation resources. Each finalizer has its own
ticket, so late cleanup cannot erase a replacement waiter at the same key.
Internal adapter/host request IDs use a checked monotonic process-local counter;
caller-generated command IDs must be fresh per invocation. These are neither
durable operation IDs nor an idempotency/deduplication journal.

Host/adapter payloads and authentication candidates never enter Machine event
history. Bounded command-status history remains available to login observers;
Core login notices use a separate typed projection, not a fabricated remote event.

## Verified telemetry binding

For each batch the existing exporter requires:

1. The exact Machine, Plugin ID, version and release digest from the private
   Service configuration loaded at startup.
2. An exact release in the currently accepted, signature-validated Catalog
   snapshot. An embedded manifest or deserialized selection is not proof.
3. The verified telemetry payload kind, contract fingerprint and matching lane
   encoding. Legacy JSONL/Prometheus and standard OTLP are distinct contracts.
4. One matching active inventory entry on that connected Machine, with no
   Provider authentication generation. Ambiguity does not pick a winner.
5. An opaque local binding that rechecks the connection, observed inventory
   revision and exact installation tuple atomically when enqueuing the command.
   Existing Machine protocol floors remain authoritative (8 legacy, 9 OTLP).

The Machine independently revalidates its installed signed release and private
export policy on execution. No endpoint or credential travels in the binding.
No version, Machine, encoding or localhost fallback is added. Local files and
incident persistence retain their independent queues and failure handling.

The later [telemetry attempt owner](telemetry-execution-leases.md) binds Machine
execution to the original connection and a monotonic admission budget, then
rechecks the exact installation and unchanged private policy at every HTTP
attempt. It is a separately deployed Machine hardening, not a durable binding
activation protocol or automatic effect restoration.

A binding expires on an observed installation identity/state/auth change,
including a switch away and back. Lease-count/detail-only observations preserve
it. The inventory revision is a conservative, connection-local observation fence,
**not** the durable `InstallationGeneration`/`InstanceIncarnation` proposed by the
design. An unobserved uninstall/reinstall cannot be distinguished by the current
wire inventory; Machine-side validation remains required.

Catalog trust follows the existing atomic refresh contract: an invalid candidate
keeps the previous accepted snapshot; successful removal prevents new resolution.
A call already resolved/enqueued may finish against its selected snapshot.
This is not instantaneous trust-file revocation or distributed lease enforcement.
Changing Service selection requires a Controller restart; Machine policy is
checked per call and may be changed independently.

## Effects and remaining work

Disposing a transport releases waiters, **not** Machine operations, workers,
worktrees or detached Sessions. A pre-enqueue binding failure is `started: false`.
After enqueue, loss of a connection/receipt is conservatively `started: true`
(may have started), unless the Machine explicitly reports preflight rejection.
There is no automatic rerouting or retry by the RPC layer. Telemetry delivery
remains best effort, may duplicate, and external emission is not reversible.

Hermetic tests cover foreign replies, reused epochs, mismatched reply kinds,
duplicate IDs, finalizer ABA, cancellation/timeout, bounded waiters, observed
inventory ABA, missing/null receipts, real signed Catalog acceptance, all three
OTLP lanes, legacy separation, policy/inventory mismatches and atomic refresh.
Temporary fixtures use no production credential or remote telemetry destination.

The [composition checker](plugin-composition-checker.md) remains read-only and
always reports `authorized: false`. General verified graph resolution, durable
typed operation intent/receipt/recovery, cross-site authorization/leases,
component codec/disposal ownership, state-version compatibility and the
security/native ownership migration remain separate unfinished design work.
This slice changes no Plugin/SDK version, Machine wire format or applied migration.
