# Installable Provider packages

Status: target architecture; the current in-tree `LaunchSpec` registry remains
the transitional implementation.

This design implements the normative ownership rules in
[Cowboy core requirements](requirements.md).

## Product boundary

A Provider is the smallest unit that a Cowboy user can discover, install,
upgrade, or uninstall. Installation is Machine-scoped:

```text
ProviderInstallation {
  machine_id
  provider_id
  provider_version
  artifact_digest
  state
}
```

The installation slot is `(machine_id, provider_id)`. A Provider release is the
immutable `(provider_id, provider_version, artifact_digest)` identified by that
slot. The same release may be installed on Hawk, Falcon, both, or neither.

Cowboy's ordinary UI exposes the Provider name, artwork, Provider version,
capabilities, Machine installation health, the Cowboy Service's single
Provider-authentication state, and Provider-owned surfaces. Authentication is
not nested under a Machine. The UI does not expose ACP, an adapter package, a
gateway, a model wire protocol, or any other transport implementation as a
separately selectable product concept. Developer diagnostics may reveal those
facts only through an explicitly scoped diagnostic surface.

`claude-code`, `codex`, `grok`, `claude-deepseek`, and `codex-deepseek` are each
independent Provider packages and installation units. A Machine may deduplicate
identical content-addressed internal blobs, but it never exposes those blobs as
installable subcomponents. Reference counting prevents one Provider uninstall
from removing a blob still leased by another Provider.

## Package ownership

Each Provider builds independently from Cowboy source against a published
Cowboy Provider SDK and contract bundle. Its artifact owns:

- its Provider descriptor and typed UI surface IR;
- logo, icon, loading, card, settings, information, empty, and error surfaces;
- its private driver and agent-launch implementation;
- exact internal executable and protocol dependency pins;
- platform-specific payloads;
- Provider-state schemas and migrations;
- its typed Service login, credential, Machine projection, refresh, revocation,
  and wipe contract;
- contract requirements and fingerprints;
- SBOM, build provenance, publisher identity, and signature.

The package may use ACP internally, but that dependency is pinned inside the
Provider. A mutable command such as `npx -y <unversioned-package>`, a moving
container tag, a dependency-owned auto-updater, or a runtime download is not a
valid released Provider. An internal dependency upgrade always produces new
artifact bytes and therefore a new Provider version.

Provider source repositories own their build and quality gates. The published
SDK supplies authoring types, the typed UI DSL, generated host bindings, the
reference verifier, conformance fixtures, and packaging tools. Cowboy repeats
all security- and compatibility-critical validation during installation; it
does not trust a Provider's build receipt.

## Typed UI SDK and component library

Cowboy publishes a versioned Provider UI component library as part of the
Provider SDK. It is a contract and authoring library, not a second copy of the
Cowboy application. Provider authors compose typed primitives and Cowboy-owned
shell slots for cards, setup, settings, information, empty, loading, error, and
session surfaces. Provider packages supply their own content-addressed logo,
icon, illustration, and loading assets through typed asset references.

The UI contract has three stages:

1. the Provider source uses generated TypeScript or Rust types and a typed DSL;
2. the Provider build compiles that source to canonical, data-only UI IR;
3. Cowboy validates the IR and renders it with its installed component library.

The package does not ship arbitrary React components, JavaScript, HTML, CSS, or
DOM access. Cowboy retains the outer card and modal shells, responsive density,
keyboard and touch semantics, focus, accessibility, theme tokens, localization,
error boundaries, and destructive-action confirmation. A Provider may arrange
approved primitives inside declared slots and constraints, so its card can have
a distinct layout without taking over the application shell.

Every component ID resolves to a versioned prop, child, event, and resource
schema. Assets declare role, media type, digest, dimensions, color variants,
motion policy, and accessible fallback. Unknown components, props, events,
tokens, asset roles, or layout constraints are type errors during Provider
build and installation errors when a package is linked by Cowboy.

## Typed linked logic

Provider UI behavior is a typed state machine rather than embedded application
code. The DSL declares:

- a closed Provider UI state schema;
- a discriminated union of messages and their payload schemas;
- pure reducers and derived values;
- typed conditions, selections, formatting, and bounded collection transforms;
- effects addressed to named Provider-driver capabilities; and
- typed success, failure, cancellation, and progress messages for each effect.

A button, selection, or lifecycle event can emit only a message accepted by
that surface. A reducer must return the declared state. Visibility, enabled,
loading, label, layout, and validation expressions must have the component
prop's expected type. Derived-value dependencies form an acyclic graph, and
state-machine matching is exhaustive. Effects cannot access credentials,
network, files, processes, or Cowboy state directly; the host mediates a
declared capability and validates both request and result.

For example, a Provider card may derive `canAuthenticate` from the typed Cowboy
Service auth state, dispatch `Authenticate { method }`, show a typed progress
surface, and reduce the Service driver's result into `ready` or `error`. A
Machine card may consume only typed replica-convergence and installation health;
it cannot dispatch a login message. The same message and state schemas are used
by the compiler, conformance tests, package linker, and runtime validator, so
the relationship is checked instead of being encoded as stringly typed event
names.

The declarative expression language should cover ordinary cross-field behavior.
If a Provider needs more complex pure computation, it may include a WebAssembly
Component that implements a versioned WIT logic world. Generated bindings make
its imports and exports typed. Cowboy gives it no DOM, ambient network,
filesystem, clock, randomness, or credential access; it runs with memory, fuel,
time, output-size, and recursion bounds. Its output remains ordinary UI IR and
is validated before rendering. This is the escape hatch for flexible logic,
not a way to load an arbitrary UI runtime.

Static authoring types cannot make an artifact downloaded months later safe by
themselves. The canonical IR, component schemas, WIT worlds, and capability
contracts are therefore the runtime type boundary. Cowboy recomputes imports,
messages, state transitions, effects, asset references, and resource bounds
from the artifact instead of trusting its declared compatibility summary.

## Artifact and compatibility

A Provider release should use one content-addressed package index with universal
UI, contract, and state layers plus one payload descriptor per supported OS and
architecture. A registry reference and an exported `.cowboy-provider` file must
resolve to the same immutable digest.

Package schema, UI IR, driver world, Controller protocol, Machine protocol, and
Provider state are independently versioned contracts. A Cowboy product version
or SDK SemVer is only a coarse filter. Installation derives requirements from
the actual UI IR and driver artifact and then performs:

1. digest, publisher signature, repository freshness, archive, and schema
   validation;
2. complete UI IR type and resource validation;
3. actual driver import/export inspection and trusted host linking;
4. required-feature and target-platform checks;
5. state-migration path validation and dry-run;
6. staged self-check and runtime conformance probes.

The Catalog may pre-filter obviously incompatible versions, but only the target
Machine can make the installation decision. Both the Controller and Machine
verify the same signed contract bundle, and the Machine independently inspects
the downloaded bytes. Compatibility uses negotiated feature sets, structural
schema checks, WIT import/export linking, and canonical contract fingerprints;
it never relies only on a package version string or a Provider-authored
`compatible: true` claim.

There are distinct failure classes: `catalog-incompatible` before download,
`artifact-invalid` for digest/signature/archive failures,
`interface-incompatible` for type or link failures, `platform-unsupported` for
a missing payload, `migration-blocked` for state, and `probe-failed` for staged
behavior. Cowboy records the machine-readable cause and renders it with a
Provider-independent host error surface; untrusted Provider UI never renders an
installation failure that occurred before activation.

An incompatible package remains quarantined and never enters the Machine's
active Provider inventory. A Service being signed out does not make an artifact
incompatible and does not change the Machine installation state. A missing or
incompatible authentication contract does.

Compatibility is bidirectional. Provider installation checks the current
Cowboy contracts, while a Cowboy Web, Controller, or Machine candidate must
check every installed and staged Provider generation before activation.

## Service-scoped authentication

Provider installation and Provider authentication are orthogonal:

```text
ProviderAuthentication {
  cowboy_service_id
  provider_id
  auth_generation
  auth_contract_fingerprint
  auth_state
  distribution_state
}

ProviderAuthReplica {
  machine_id
  provider_id
  auth_generation
  replica_state
  materialization_state
}
```

Cowboy performs the Provider's login flow once at Service scope. The resulting
portable credential bundle is encrypted in a Service-owned vault and assigned a
monotonic `auth_generation`. Ordinary database rows contain only redacted state,
account labels, schema fingerprints, generation numbers, and convergence
receipts; the vault key and plaintext credential do not enter the database.

The Service automatically distributes the current generation to every enrolled,
authorized, non-revoked Machine. A bundle is sealed to that Machine's enrollment
public key and stored under a private Service-managed replica root. Every
Machine verifies and acknowledges the signed envelope and generation without
requiring the Provider to be installed. A Machine with the Provider installed
also validates the typed projection schema, atomically materializes the official
runtime state, probes it, and acknowledges materialization. Neither receipt can
become an independent account or login state.

Online Machines reconcile as part of login. Offline Machines retain `pending`
convergence and reconcile immediately after reconnect. New Machines receive the
current generation after enrollment. Installing a Provider on a Machine that
already has the sealed replica automatically materializes and probes it; no
second login is shown.

The Service owns auth `signed_out`, `authenticating`, `ready`, `expired`, or
`error`, plus aggregate distribution `idle`, `distributing`, `converged`,
`degraded`, or `revoking`. Machine diagnostics separate replica `pending`,
`storing`, `current`, `failed`, or `revoking` from materialization
`not-installed`, `applying`, `current`, or `failed`. An offline Machine degrades
distribution without changing the Service's authenticated state. A Provider
that requires auth is schedulable on a Machine only when Service auth is ready
and that Machine has materialized and probed the current generation.

The package authentication contract must make refresh deterministic. It either
keeps refresh in the Service and issues bounded Machine projections, or returns
a sealed candidate through a generation compare-and-swap. Cowboy validates the
candidate, commits one next generation, and redistributes it. Two Machine
replicas may never silently become different accounts.

When an upstream credential is non-exportable or device-bound, the Provider
must supply a safe Service broker or token-exchange projection. Otherwise the
Provider is incompatible with the Service-auth contract. Cowboy does not expose
a per-Machine login escape hatch.

Service logout immediately blocks new matching sessions, invalidates the active
generation, and distributes a signed wipe or revocation generation. Offline
Machines apply it on reconnect and cannot start new work while stale. Uninstall
on one Machine removes its materialized Provider credential state but preserves
the Service login and other Machine replicas.

## Catalog and Machine installation

Publishing and installation are separate state transitions:

```text
Provider source
  -> verified immutable release
  -> signed Provider Catalog entry
  -> available version in Cowboy UI
  -> user selects Machine and version
  -> that Machine stages, probes, and activates the Provider
```

The Catalog records releases; it does not imply installation. The Controller
joins Catalog releases with each Machine's platform, contract inventory, active
installation, and health. The UI therefore presents Provider versions inside a
specific Machine context and reports `available`, `installing`, `active`,
`upgrade-available`, `incompatible`, or `uninstalling` without exposing the
internal transport. Provider authentication appears once at Service scope;
replica lag is installation health or developer diagnostics, never a Machine
login control.

The UI submits an immutable Catalog reference or uploads an artifact. It never
supplies an arbitrary Machine download URL. Installation follows a durable
state machine:

```text
resolving -> quarantined -> verified -> interface-checked
          -> staged -> probed -> active
```

Failure before `active` leaves the prior generation unchanged. A Provider
upgrade installs side by side; existing sessions stay pinned to their original
Provider generation, while new sessions use the newly active generation.
Activation always links the authentication projection contract. When a current
Service auth generation exists, staging also materializes and probes it; a
signed-out Service does not block installation. Scheduling waits for the current
generation's materialization acknowledgement when auth is required.

## Sessions and uninstall

Every session persists `provider_id`, `provider_version`, and
`provider_generation_digest`. Provider name matching is insufficient because
multiple generations may coexist.

Uninstalling a Provider from one Machine affects only sessions pinned to that
Machine and Provider installation. The confirmation plan includes the exact
session set, active-turn count, retained data classes, and absolute purge time.
On confirmation Cowboy atomically blocks new sessions, soft-deletes the impact
set, removes it from ordinary UI, drains workers, and releases the installation
lease. Session records use an absolute `purge_after_at` so later policy changes
cannot move an already confirmed deadline.

Uninstall wipes that Machine's materialized Provider credential state after the
worker leases drain. Its Service-managed sealed replica remains synchronized but
cannot be opened by an absent Provider; this lets reinstall consume the current
generation without another login. Only Service logout, Machine revocation, or
credential-generation retirement wipes the sealed replica. Uninstall does not
log the Cowboy Service out or affect another Machine's replica.

The confirmation modal names the Machine and Provider, separates sessions that
are idle from turns that must drain or be cancelled, states that source projects
and user worktrees remain untouched, and shows the calendar date when Cowboy's
retained session data becomes eligible for permanent deletion. A second modal
is required if policy allows cancelling active turns. Reinstalling the Provider
does not silently undelete sessions; recovery, when policy permits it, is an
explicit operation before `purge_after_at`.

Project source and session worktrees are not Provider package data and are not
deleted by default. Cowboy may reclaim its own generated build artifacts after
it proves the owning worker exited. Shared content-addressed Provider blobs are
removed only after their final lease and rollback or recovery deadline expire.

## Dependency upgrade and release

Use the repository skill
[`release-cowboy-provider`](../.agents/skills/release-cowboy-provider/SKILL.md)
to audit a Provider's internal dependencies, upgrade exact pins, run the
Provider and Cowboy contract gates, build and sign a new immutable artifact,
publish it, and verify that the Catalog advertises its digest.

The release workflow stops at Catalog availability. Installing or upgrading the
new version on a Machine is a separate user action in Cowboy UI. A bulk audit
may inspect every Provider, but each Provider retains an independent checkout,
commit, version, artifact, signature, test receipt, and release transaction.

## Migration from the in-tree registry

The current implementation still defines Provider launch recipes in
`src/provider/mod.rs`, exposes a static Web Provider list, and resolves internal
agent packages at worker launch. Migration should proceed without pretending
the target package model already exists:

1. publish the Provider SDK, package schema, authentication contract, trusted
   verifier, and Catalog contract;
2. add Service-owned auth generations and automatic replica reconciliation,
   then retire Machine login UI after end-to-end convergence;
3. add persistent Machine-scoped Provider installations and generation-pinned
   session identity;
4. convert one Provider to a fully pinned package and prove install, Service
   login synchronization, refresh, upgrade, rollback, uninstall, logout, and
   session retention end to end;
5. migrate the remaining Providers independently;
6. remove static Provider UI tables and in-tree launch ownership only after the
   final built-in generation has drained.

Until those prerequisites exist, editing an in-tree `LaunchSpec` or releasing a
Cowboy component is not a Provider release.
