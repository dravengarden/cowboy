# Core Service-bound Plugin Sites

The existing finite Plugin transports now bind their declared Service/Machine
Site to the Controller's established Service identity and authenticated Machine
route. This extends [verified release observations](plugin-release-leases.md)
and [finite telemetry resolution](resolved-telemetry-ports.md); it does not turn
a composition proposal into an authorized graph.

## Owner and resolution

Core startup loads or creates the existing `service-id` and obtains a private
`ServiceIdentity`. The production type has no string constructor, default, Clone
or serde implementation. `MachineControl` must consume that identity; it can no
longer create a production registry without a Service owner. Public identity
projections, configuration-only inspection and wire fields remain data, not
constructors for the owner.

The identity names a logical Service, not a principal, live process, exclusive
database writer or authorization. The existing Controller owner lock, identity
file format, Operator checks and enrollment remain independent and unchanged.
Reopening the same Service preserves its name, but a new authenticated channel
has a new connection identity even with identical Machine and epoch strings. It
does not inherit an old transport handle or end a detached Session.

Telemetry binding and OTLP resolution first require this exact Service owner and
Machine route, before acquiring Catalog or installation observations. Revocation
still needs neither a removed release nor its installation; it does need the
correct Service. Installation, uninstall and recovery preflights also compare
both identity axes. Another Service's identically named Machine is not the same
Site.

## Final dispatch boundary

Both outgoing paths, `send` and RPC registration/enqueue, check the complete
claimed Site against the immutable owner and actual route under their existing
channel lock. A generic call cannot bypass this check. Mismatches are rejected
before creating a pending waiter or sending bytes, with bounded errors that do
not print input identities or payloads.

The exhaustive core classification covers all twelve scoped command variants:
installation observation/apply/query; uninstall apply/query/recovery query;
telemetry binding query/commit; recovery apply/query/audit query; and managed
export. Read-only history is subject to the same Site isolation, without
renewing deadlines or requiring a still-installed Plugin. Adding a command
requires explicit classification at compile time.

Existing wire commands without Service/Machine fields retain their own domain
and authenticated-transport checks. Core does not inspect opaque adapter JSON or
encrypted credentials to invent a scope. This is not a new generic mutation API,
an authorization grant, an atomic cross-Site commit, or a claim that all legacy
commands have acquired a new scoped wire contract. Machine independently
continues to check enrolled ownership, exact installation/CAS, budget and
policy.

## Evidence and limits

Three regression tests failed before the repair: a foreign-Service revoke was
enqueued, an installation read awaited a remote reply, and a binding obtained a
resolved port. Source tests additionally exercise all twelve command variants
across both Service/Machine identity axes through direct, generic RPC and
original-connection RPC entrypoints; wrong-Site paths send nothing and retain no
waiter. They also cover all three OTLP signals, release-free revoke, separate
Services with identically named Machines, and reopen without handle adoption.
These are temporary identities, signed fixtures and disposable channels, not
production Operator authority or an externally exploitable HTTP-route finding.

No Plugin/SDK/component release, Machine protocol, SQL migration, durable
journal, identity-file format or production policy changes. The
[Controller release](releases/plugin-service-sites-2026-09-15.md) passed
complete source gates and all 807 immutable role checks and is published and
active on Hawk. Its bounded activation window retained Machine, 15 workers and
Victoria; the pre-existing GTK portal failed state was preserved, not cleared.

Workspace/Session/security-domain resolution, general state leases, graph
contract linking and independent post-effect/native restoration remain in the
[completion ledger](plugin-refactor-completion.md). In particular, this finite
Site check does not make Agent-internal effects reversible, restore a turn or
authorize a managed Victoria cutover.
