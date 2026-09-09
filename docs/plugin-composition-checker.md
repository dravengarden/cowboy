# Read-only composition checker

Implemented first slice of the
[target architecture](plugin-spatiotemporal-design.md). This is a **core
diagnostic**, not a Plugin, an install format, an executable workflow, or a
replacement for the current Catalog/authorization protocol.

## Try it

From the repository root, in the pinned shell:

```sh
nix develop -c cargo run --locked --bin cowboy -- composition check contracts/examples/victoria-service-machine.json
nix develop -c just composition-check
```

The example uses synthetic release digests and contract fingerprints. It places
a Service-scoped Victoria implementation on Machine `hawk` and connects a
Service telemetry ingress to its remote port. It neither installs Victoria nor
changes the existing exporter. OTLP bodies retain their standard protobuf wire
format; the proposal contains only port metadata, never telemetry or secrets.

The command reads one bounded regular file and prints a JSON structural report.
It does not query the network, credentials, Catalog, inventory, policy or store.
Bad input exits nonzero with a bounded error code, without echoing input fields.
The report always says `status: "structurally_valid", authorized: false`. Its
type has no deserializer and no public constructor; no executor accepts it.

## Implemented boundaries

- One closed JSON Schema 2020-12 profile generates Rust newtypes/enums and
  TypeScript branded/discriminated types plus shape/refinement validators. The
  only editable wire source is
  [`contracts/composition-v1.schema.json`](../contracts/composition-v1.schema.json).
  Generated Rust stays private to core; this does not publish a new Plugin SDK.
- JSON decoding rejects duplicate keys (including escaped aliases), invalid
  Unicode, trailing data, unsafe JSON numbers, and input/depth budget excesses.
  Records and tagged unions reject unknown fields. Required values cannot be
  omitted or replaced by `null`.
- Revision/generation/incarnation values are canonical decimal **strings** in
  `0..=18446744073709551615`. IDs and version/digest refinements are bounded
  ASCII. These checked syntax types are not proof of enrollment, installation or
  trust.
- A proposal has exactly one logical Service root. Machine, Workspace, Session
  and Operation scopes have explicit parent constraints; duplicate logical scope
  aliases, missing parents and cycles fail. Concrete scope ancestry is checked,
  not just scope kind.
- Service scope may run on a Machine. A Machine-affine scope cannot silently
  move to another Machine or the Controller. All sites name the same Service.
  Plugin `isolated` execution on a Service is rejected; a data facet is allowed.
- Core anchors and Plugin instance identities are disjoint union branches. The
  same installation slot/generation cannot name two exact releases. Old and new
  generations may coexist; this check does **not** assert state compatibility.
- Ports bind explicit node/port endpoints and exact contract
  ID/version/fingerprint. `one`, `optional` and `many` cardinalities never pick
  a provider by registration order. Local references cannot cross sites.
  Descendant visibility is explicit; sibling Sessions cannot see each other's
  scoped providers.
- Node ownership requires the same site and an equal/ancestor lifetime.
  Ownership prerequisites and capability dependencies are cycle-checked
  **together**.
- Limits: 1 MiB input, JSON depth 32, 256 scopes/nodes each, 32 ports in each
  provided/required list, 2,048 total ports and 2,048 bindings. A component
  cannot multiply the total budget.

The report includes deterministic provider/owner-before-consumer ordering,
per-site projections and explicit cross-site bindings. The reverse order is a
dependency diagnostic, **not rollback or proof that an effect is reversible**.

## Identity and generation

`just composition-generate` refreshes both generated files. `composition-check`
and the complete `just check` gate reject stale output and run shared wire
acceptance vectors in Rust/TS. Graph semantics currently run in Rust only; a TS
decoder success is not a graph-validation success.

Contract fingerprint v1 hashes `cowboy.closed-contract.v1\n` followed by compact
JSON with recursively sorted object keys and unchanged array order. It includes
the schema and its profile/budget/refinement annotations. This generator admits
only its closed, non-recursive, local-reference profile and common ASCII regex
subset; it is not a general JSON Schema compiler. Profile changes require review
and cross-runtime vectors.

Proposal digest v1 hashes `cowboy.composition.proposal.v1\n`, the contract
fingerprint, a newline, then compact recursively key-sorted proposal JSON.
Scopes, nodes and port lists sort by ID; bindings sort by consumer endpoint,
provider endpoint and revision. `many` uses that explicit stable endpoint order.
JSON property order and graph declaration order do not alter the digest;
changing an exact identity, generation, binding revision or constraint does.

Neither digest is a signature or a grant. Port metadata and release references
are still untrusted declarations. Actual resolution must compare them with
verified packages, current enrollment, grants, policy epochs and live leases.

## Still to implement

This completes a runnable structural foundation, not the entire P0–P4 design.
Next are verified runtime inventory resolution and typed operation
intent/receipt contracts, then component codec/disposal ownership and
SDK/host/native separation. The current coordinated component/Plugin release
rule remains in force.

There is no cross-site activation, durable operation journal, recovery executor,
state reader/writer compatibility enforcement or per-call revocation in this
checker. Existing authentication, Machine protocol, session generations, native
bridge, Plugin versions and deployed migration bytes remain unchanged. UI
unmount and Controller restart have not acquired permission to cancel detached
sessions.
