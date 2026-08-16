# Cowboy core requirements

Status: normative Provider platform contract. Package schema 2, release schema
2, UI schemas 1-2, host integration schemas 1-2, Provider SDK 2.4,
Machine-scoped installation, exact session generations, Service-scoped
authentication, and bounded uninstall retention implement this boundary. The
in-tree `LaunchSpec` registry remains only as a drain-compatible fallback for
sessions created before an exact Provider generation was recorded.

## Authority

These requirements govern new Provider-platform work. When a target design
choice conflicts with a transitional implementation detail, preserve the live
system until a tested migration exists, then move toward this contract. Do not
silently redefine the contract to match the transition.

The detailed package and type-system design lives in
[Installable Provider packages](provider-packages.md). Runtime details live in
the numbered [architecture chapters](architecture/00-overview.md).

## State ownership

| State | Owner and identity |
|---|---|
| Provider release | Catalog: `(provider_id, provider_version, artifact_digest)` |
| Provider installation | Machine: `(machine_id, provider_id)` |
| Provider authentication | Cowboy Service: `(cowboy_service_id, provider_id)` |
| Authentication replica | Derived Machine copy: `(machine_id, provider_id, auth_generation)` |
| Session runtime | `(session_id, machine_id, provider_id, provider_generation_digest)` |

An authentication replica is not a Machine login. It is a versioned projection
of the Cowboy Service's authoritative Provider authentication state.

## Product requirements

### CR-1: Cowboy is Provider-agnostic

Cowboy core owns hosting, lifecycle, persistence, scheduling, component
rendering, security, and typed extension contracts. It must not require a new
agent-specific card, icon branch, loading branch, install flow, settings form,
or runtime protocol branch when a conforming Provider is added.

ACP, an adapter, gateway, agent CLI, model protocol, and their versions are
Provider-private implementation details. Ordinary UI presents the Provider;
only explicitly scoped developer diagnostics may name those internals.

### CR-2: Provider is the product and installation unit

`claude-code`, `codex`, `gemini`, `grok`, `claude-deepseek`, and
`codex-deepseek` are independent Providers. Each builds, versions, signs,
publishes, installs, upgrades, rolls back, and uninstalls independently.

A user installs a Provider version on a selected Machine. Cowboy never asks the
user to install an ACP runtime, adapter, gateway, managed Node, or CLI as a
separate product component. Internal content-addressed blobs may be deduplicated
and leased without changing this UI or lifecycle boundary.

### CR-3: Every release is immutable and independently buildable

A Provider builds independently against the published Cowboy Provider SDK and
contract bundle. First-party sources may be co-located in this repository, but
each has its own source manifest, version, build invocation, artifact, release
envelope, signature, and install transaction. Its data-only package includes
all universal UI and contract layers; its signed release binds exactly one
runtime-artifact set for every declared platform. Its identity binds Provider
ID, Provider version, package digest, composite artifact digest, contract
fingerprint, runtime-artifact matrix, and publisher.

Every internal executable and protocol dependency is pinned to an exact version
and digest inside the Provider. Moving tags, version ranges, `latest`, runtime
package installation, and dependency-owned auto-updaters are forbidden. Any
dependency change creates a new Provider version and new artifact bytes.

If a Provider needs a gateway or another auxiliary runtime, it declares a
closed, typed session-sidecar contract. The worker allocates a unique loopback
endpoint, launches the exact generation-local component, waits for its declared
health path, and resolves signed `sidecar_url` bindings only after readiness.
Component executable bindings are resolved from the same installed generation.
No fixed Machine-global gateway, host profile path, shell template, or ambient
runtime download may complete an installable Provider release.

### CR-4: Cowboy supplies the typed component library

The Provider SDK exposes typed Cowboy components and shell slots for logos,
icons, loading, cards, setup, settings, information, empty, error, and session
surfaces. A Provider may compose and lay out approved primitives inside those
slots. Cowboy retains the application shell, responsive behavior, keyboard and
touch semantics, focus, accessibility, theme, localization, error isolation,
and destructive confirmation.

Providers do not inject React, JavaScript, HTML, CSS, or DOM code. Authoring
types compile to canonical data-only UI IR that Cowboy validates and renders.
Provider-specific activity uses a closed indicator union and typed label
effects; Provider-specific vector color uses bounded gradient records. Cowboy
owns the animation implementation, reduced-motion behavior, accessibility, and
responsive mechanics. The host renderer must dispatch only on schema kinds,
never on Provider IDs.

Provider-specific Transcript thoughts use a separate closed presentation
contract. A Provider selects one Cowboy-owned `timeline`, `workcell`, `signal`,
or `terminal` variant, a bounded density, an optional active label, and a
`plain` or `soft` current-step surface. Cowboy owns marker geometry, typography,
color application, motion, reading-scale behavior, and accessibility. The
contract cannot carry CSS, DOM, markup, arbitrary dimensions, or executable
rendering logic.

Service authentication presentation follows the same boundary. Cowboy derives
the closed `account` or `api_key` presentation from the Provider's typed auth
method union. Account Providers render sign-in language; Providers whose only
declared methods are `secret_input` render API-key status, actions, and modal
copy. Cowboy owns card geometry and interaction mechanics, and Web must not
branch on a Provider ID to choose either presentation.

Ordinary Web renders `information` and `setup` once in a Cowboy Service
authentication region, while each Machine renders only `card` and its
`empty`/`settings` lifecycle surface. An installed Machine card is rendered from
that exact installed package; a newer Catalog entry is only the upgrade target.
Runtime-advertised configuration options remain protocol data, but any
Provider-specific ordering, full-width layout, or session-lifecycle
availability is declared in that signed package's typed
`configuration.options` contract. Cowboy UI may supply neutral defaults for
portable option concepts; it must not branch on a Provider-specific option ID.
Provider-specific tool rendering follows the same boundary: the exact package
may map an upstream tool name to a closed Cowboy renderer through
`host.tool_presentations`; Cowboy Web must not keep a Provider-ID/tool-name
dispatch table.

### CR-5: Linked behavior remains type-safe

Provider UI behavior uses closed state schemas, typed messages, pure reducers,
derived expressions, and capability-mediated effects. Component props, event
payloads, reducer results, effects, and effect results are checked during build
and checked again when Cowboy installs the artifact. Configuration-option
presentation records are a closed union with unique typed option IDs, bounded
order values, layout, and availability policies; malformed or duplicate rules
fail both Rust package validation and TypeScript Catalog validation. Tool
presentation declarations likewise use unique bounded names and a closed
renderer union.

Host integration schema 1 forbids Transcript presentation fields. Host schema
2 requires Transcript presentation schema 1, validates its exact field set and
closed tokens, and includes it in the UI contract fingerprint. Unknown host or
Transcript schema versions, variants, tokens, fields, control characters, and
oversized active labels fail in both Rust and TypeScript validation.

The UI schema version is part of the runtime type boundary. Cowboy accepts only
an explicitly supported interval, rejects fields introduced after the declared
schema version, and validates every activity frame, phrase, interval, asset
link, gradient stop, and responsive layout constraint in both the Rust package
validator and TypeScript Catalog validator. A new union member requires a new
schema version and host implementation; unknown members fail closed.

Effect ownership is also structural: Service authentication and logout may be
emitted only from `setup`, Machine installation only from `empty`, and Machine
upgrade or uninstall only from `settings`. Documentation links are the only
lifecycle-neutral effect. Both Rust package validation and TypeScript Catalog
validation enforce the same mapping before rendering.

The current schema has no executable logic escape hatch. Logic that exceeds the
closed DSL requires a future versioned SDK schema and corresponding Cowboy host
implementation; a Provider cannot inject JavaScript, WebAssembly, or ambient
DOM, credential, network, process, filesystem, clock, or randomness access.

### CR-6: Compatibility is derived and fail-closed

SDK SemVer and a Provider-authored compatibility interval are only pre-filters.
The SDK pre-filter accepts the same major only and rejects a Provider built
against a newer SDK than the validating Cowboy host. Cowboy then derives
requirements from the actual UI IR, assets, state machines, effects, host
profiles, runtime commands, exact dependency links, platform matrix, and
authentication contract.

Runtime arguments and environment values are either bounded literals or closed
host bindings to a declared private component command or declared session
sidecar URL. Sidecar IDs, component kinds/slots, loopback transport, readiness,
credential-environment forwarding, and every cross-reference are validated in
both SDK implementations. Unknown, dangling, or platform-incomplete links fail
before installation.

Build, Catalog ingestion, and Machine installation validate schema versions,
structural types, closed component/capability enums, canonical fingerprints,
Controller/Machine contract intervals, complete platform/component binding,
package and composite digests, publisher signature, URL/archive bounds, and
probes. The target Machine independently inspects downloaded bytes. An
incompatible artifact never replaces the active generation.

Each Machine advertises a strict `ProviderContractInventory` in its signed
hello: Provider SDK version, supported package/release/UI/host schema intervals,
Machine contract, platform, and architecture. The Controller repeats the same
compatibility predicate immediately before an install or upgrade, and Web may
offer only the newest ready release accepted by that inventory. An absent,
malformed, or insufficient inventory fails closed with a typed compatibility
code and an update-Machine explanation; Cowboy must not send a newer package to
an older decoder and expose its raw deserialization error.

### CR-7: Installation is dynamic and Machine-scoped

The Cowboy UI joins Catalog releases with one Machine's platform, contracts,
current installation, and health. Install and upgrade use immutable references,
stage side by side, probe before activation, preserve the prior generation on
failure, and retain leased generations for existing sessions and rollback.
Each session launches auxiliary components from its exact retained generation
on its own dynamic loopback endpoint. Old and new Provider or authentication
generations may therefore drain concurrently without sharing a fixed port or
silently adopting replacement runtime bytes.

Publishing a Provider release does not install it. Installing it on one Machine
does not install it on another Machine.

Install and uninstall serialize per `(machine_id, provider_id)`. Installation
validates any current Service auth envelope before changing activation and
restores the previous runtime and auth links if the commit fails. Reusing a
retained generation re-hashes its stored runtime artifacts against the signed
release matrix rather than trusting cache metadata or executable existence.

### CR-8: Authentication is Cowboy Service-scoped

Cowboy performs one Provider login for the entire Cowboy Service. Ordinary UI
must not offer Machine-specific Provider login, logout, account selection, or
credential entry. A successful login creates a monotonic `auth_generation` in a
Service-owned encrypted credential vault; the vault key is not stored in the
ordinary Cowboy database.

Authentication starts against an exact signed `(provider_id, version,
artifact_digest, auth_contract_fingerprint)` and may use only a connected
Machine with that exact release active as its temporary executor. The returned
candidate must repeat the same immutable identity before Service commit; an
upgrade race or mismatched method fails closed.

The Service Catalog advertises the deduplicated signed release identities that
are active on connected Machines without exposing Machine identity. The UI may
render newer compatible presentation assets, but it starts a selected method
with the newest advertised exact release that declares that method. If no such
executor exists, the UI must show an actionable install-or-upgrade error rather
than swallowing the failed request or weakening exact-release validation.

Cowboy automatically reconciles that generation to every enrolled, authorized,
non-revoked Machine. Each credential bundle is sealed to the target Machine's
enrollment key, stored in a private Service-managed replica area, and exposed
only to the matching Provider runtime. Every online Machine acknowledges sealed
replica storage; a Machine with the Provider installed additionally acknowledges
typed materialization. The Service reports distribution `current` only after
applicable acknowledgements; partial reachability is `partial`. Offline Machines
remain pending and reconcile automatically on reconnect. A newly enrolled
Machine receives the current generation without another login.

The Service owns auth state `signed_out`, `authenticating`, `ready`, `expired`,
or `error`, plus distribution `none`, `pending`, `current`, `partial`, `failed`,
or `revoking`. A Machine may report only sealed-replica convergence
such as `pending`, `storing`, `current`, `failed`, or `revoking`, plus
materialization `not-installed`, `applying`, `current`, or `failed`. Those are
not login states. An offline Machine may degrade distribution without changing
the Service from authenticated `ready` or blocking a current online Machine.

An active login and its pre-commit error are Service-wide transient status, not
credential generations. They may report generation zero before the first
successful commit, are visible through the Catalog, and are never sealed or
replicated. Scheduling and Machine synchronization consult only a durable,
validated generation.

Each Provider declares a typed authentication contract: login flow, portable
credential schema, Machine projection schema, validation, import, refresh,
revocation, and wipe behavior. Refresh either remains Service-owned with
short-lived Machine projections or returns a sealed candidate through a
compare-and-swap generation update that the Service validates and redistributes.
Replicas may not drift into independent accounts.

If upstream credentials are non-exportable or Machine-bound, the Provider must
implement a safe Service broker or token-exchange projection. Otherwise it is
incompatible with Cowboy Service authentication; Cowboy must not fall back to
per-Machine manual login.

Service logout blocks new Provider sessions immediately, invalidates the active
authentication generation, and automatically distributes a signed wipe or
revocation generation. Provider uninstall on one Machine wipes that Machine's
materialized Provider state but does not log out the Cowboy Service.

### CR-9: Sessions bind exact generations

Every session persists Machine ID, Provider ID, Provider version, and Provider
generation digest. Existing sessions may drain on their leased generation while
new sessions use the active generation. Provider names or mutable active links
are never sufficient session identity.

A new session is schedulable only when its Machine has the Provider installed,
the required Service authentication is ready, and the Machine has acknowledged
materializing and probing the current authentication generation.

### CR-10: Uninstall is explicit and recoverable for a bounded period

Uninstalling `(machine_id, provider_id)` blocks new matching sessions, identifies
the exact impacted session and active-turn set, soft-deletes those sessions from
ordinary UI, drains or explicitly cancels workers, and releases the installation
lease. It does not affect that Provider's sessions on another Machine.

The confirmation modal names the Machine and Provider, counts idle and active
sessions, distinguishes drain from cancellation, lists retained data classes,
states that source projects and user worktrees remain untouched, and displays
the absolute `purge_after_at` date. Permanent cleanup runs only after that date.
Reinstall does not silently restore soft-deleted sessions. Hard deletion
cascades session events; content-addressed event attachments are reference
scanned and deleted only when no retained event references them and their
race-avoidance grace period has elapsed.

If an ordinary Machine command or database transaction fails after uninstall
has begun, Cowboy compensates by re-verifying and reactivating the exact retained
signed generation and restoring workers that were live before the operation.
The operation reports both the primary and compensation failure when recovery
cannot complete; it must never report a successful uninstall with only half of
the Machine/session state committed.

### CR-11: Release automation belongs to this repository

The canonical Provider dependency-audit and release procedure is the repository
skill at
[`../.agents/skills/release-cowboy-provider/SKILL.md`](../.agents/skills/release-cowboy-provider/SKILL.md).
It is versioned, reviewed, and released with Cowboy's Provider contracts. Do not
fork or hand-maintain it as a user-home skill.

The skill audits internal dependency updates, updates exact pins, runs Provider
and Cowboy conformance gates, builds and signs one immutable Provider release,
publishes it, and verifies Catalog availability. It stops before Machine
installation and never performs Service login as a release side effect.

### CR-12: Trust is re-established at every boundary

A successful Provider build receipt is evidence, not installation authority.
Cowboy verifies publisher trust, package and composite digests, signature,
archive safety, interfaces, capabilities, complete platform/runtime binding,
exact dependency versions, and probes again. Credentials, login codes, tokens,
and credential-bearing state never enter UI IR, logs, telemetry, Catalog
metadata, Provider artifacts, or ordinary Cowboy database rows.

## Minimum acceptance suite

The Provider platform is not complete until automated acceptance proves:

- each Provider builds independently from clean source against an exact SDK
  contract without depending on another Provider's source or artifact;
- malformed, mistyped, over-capable, unsigned, incompatible, and wrong-platform
  packages fail before activation;
- a release can install, upgrade, roll back, and uninstall independently while
  existing sessions retain their exact generation;
- install/uninstall races are fenced, corrupt retained runtime bytes fail
  reactivation, and a returned database/Machine failure restores the exact
  pre-uninstall generation and live-session set;
- one Cowboy Service login automatically converges the same authentication
  replica to every connected Machine, materializes it on every installed
  Provider, and later converges it to an offline or newly enrolled Machine;
- concurrent credential refresh cannot fork the authoritative generation;
- Service logout blocks new sessions and causes every Machine replica to be
  wiped or marked pending revocation until it reconnects;
- uninstalling one Machine's Provider removes only its affected sessions and
  does not delete Service authentication, source projects, or worktrees; and
- session hard purge removes cascaded events and later reclaims only truly
  unreferenced event attachments.

## Implemented migration boundary

Provider package schema 2, release schema 2, UI schemas 1-2, host integration
schemas 1-2, Controller contract 2, Machine contract 4, and Cowboy Provider SDK
2.4 in both Rust and TypeScript are the active contract.
The Catalog embeds the six independently compiled first-party manifests as
typed `unbound` entries and accepts installable releases only after an external
`.cowboy-provider` package is paired with a complete, signed runtime envelope.
Target Machines repeat package, composite digest, publisher, contract,
platform, private dependency, archive, and staged-probe checks before
atomically changing their active generation.

Exact package workers receive a Machine-verified command map for every private
component. Cowboy prepends only those generation-local directories to the
worker path, resolves component-command bindings without a shell, starts
declared gateways as session-owned sidecars on dynamic loopback ports, requires
their typed readiness probes, and keeps their process handles for the complete
worker lifetime. This makes the Provider, rather than a Machine-global adapter,
CLI, gateway, or resource path, the executable installation unit.

Web discovers Providers from `/api/providers`, renders their closed UI IR and
assets, and exposes only Provider-level install, upgrade, authentication, and
uninstall actions. ACP, adapters, gateways, and managed CLI components remain
available only to developer diagnostics. The Service authentication surface is
rendered once outside Machine cards; its lifecycle details and each Machine's
Provider lifecycle details are collapsed behind compact summaries by default.
Machine cards use the exact installed manifest and treat the latest ready
Catalog release only as an upgrade target. Session execution stays pinned to
its exact generation; generic shell chrome may adopt the latest compatible
signed presentation for the same Provider when an exact session uses an older
UI or host-presentation schema and would otherwise retain stale brand, activity,
or Transcript UI.
Sessions persist exact Provider and auth generations. Service auth uses one
encrypted, monotonic durable generation plus a Service-wide transient login
status, and automatically seals durable state to enrolled Machines; offline
replicas remain pending and converge on reconnect. Uninstall uses an expiring
exact-impact plan, active
turn confirmation, an absolute three-day purge deadline, and reference-aware
attachment cleanup. Machine protocol 3 carries Provider/auth lifecycle
commands; protocol 4 adds exact retained-generation reactivation for uninstall
compensation. Provider lifecycle fences serialize install/uninstall, auth
generations reject stale or conflicting replicas, and session launch resolves
the exact recorded auth projection.

The legacy in-tree launch registry is retained only to drain old sessions that
lack an exact package generation. New Machine-backed sessions must resolve an
active signed Provider package. Remove that fallback after the last legacy
session generation is no longer restorable.
