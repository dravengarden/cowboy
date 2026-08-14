# Cowboy core requirements

Status: normative target contract. Architecture chapters that describe the
running in-tree Provider registry or Machine-scoped login remain operationally
accurate during migration, but they are not the target product boundary.

## Authority

These requirements govern new Provider-platform work. When a target design
choice conflicts with a transitional implementation detail, preserve the live
system until a tested migration exists, then move toward this contract. Do not
silently redefine the contract to match the transition.

The detailed package and type-system design lives in
[Installable Provider packages](provider-packages.md). Current runtime behavior
lives in the numbered [architecture chapters](architecture/00-overview.md).

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

`claude-code`, `codex`, `grok`, `claude-deepseek`, and `codex-deepseek` are
independent Providers. Each builds, versions, signs, publishes, installs,
upgrades, rolls back, and uninstalls independently.

A user installs a Provider version on a selected Machine. Cowboy never asks the
user to install an ACP runtime, adapter, gateway, managed Node, or CLI as a
separate product component. Internal content-addressed blobs may be deduplicated
and leased without changing this UI or lifecycle boundary.

### CR-3: Every release is immutable and independently buildable

A Provider builds outside the Cowboy source tree against a published Provider
SDK and contract bundle. Its package includes all universal UI and contract
layers plus every declared platform payload. Its identity binds Provider ID,
Provider version, artifact digest, publisher, and provenance.

Every internal executable and protocol dependency is pinned to an exact version
and digest inside the Provider. Moving tags, version ranges, `latest`, runtime
package installation, and dependency-owned auto-updaters are forbidden. Any
dependency change creates a new Provider version and new artifact bytes.

### CR-4: Cowboy supplies the typed component library

The Provider SDK exposes typed Cowboy components and shell slots for logos,
icons, loading, cards, setup, settings, information, empty, error, and session
surfaces. A Provider may compose and lay out approved primitives inside those
slots. Cowboy retains the application shell, responsive behavior, keyboard and
touch semantics, focus, accessibility, theme, localization, error isolation,
and destructive confirmation.

Providers do not inject React, JavaScript, HTML, CSS, or DOM code. Authoring
types compile to canonical data-only UI IR that Cowboy validates and renders.

### CR-5: Linked behavior remains type-safe

Provider UI behavior uses closed state schemas, typed messages, pure reducers,
derived expressions, and capability-mediated effects. Component props, event
payloads, reducer results, effects, and effect results are checked during build
and checked again when Cowboy installs the artifact.

Complex pure logic may use a resource-bounded WebAssembly Component with a
versioned WIT world and generated bindings. It receives no ambient DOM,
credential, network, process, filesystem, clock, or randomness access. Its
output is still validated UI IR.

### CR-6: Compatibility is derived and fail-closed

SDK SemVer and a Provider-authored compatibility claim are only Catalog
pre-filters. Cowboy derives requirements from actual UI IR, assets, state
machines, effects, driver imports and exports, platform payloads, migrations,
and authentication contracts.

Both build and installation validate schema versions, structural types,
component contracts, WIT linking, capability sets, canonical fingerprints,
resource bounds, platform support, migration paths, signatures, and provenance.
The target Machine independently inspects downloaded bytes and runs staged
probes. An incompatible artifact remains quarantined and never replaces the
active generation.

Cowboy Web, Controller, and Machine upgrades perform the reverse check against
all active, staged, and session-leased Provider generations before activation.

### CR-7: Installation is dynamic and Machine-scoped

The Cowboy UI joins Catalog releases with one Machine's platform, contracts,
current installation, and health. Install and upgrade use immutable references,
stage side by side, probe before activation, preserve the prior generation on
failure, and retain leased generations for existing sessions and rollback.

Publishing a Provider release does not install it. Installing it on one Machine
does not install it on another Machine.

### CR-8: Authentication is Cowboy Service-scoped

Cowboy performs one Provider login for the entire Cowboy Service. Ordinary UI
must not offer Machine-specific Provider login, logout, account selection, or
credential entry. A successful login creates a monotonic `auth_generation` in a
Service-owned encrypted credential vault; the vault key is not stored in the
ordinary Cowboy database.

Cowboy automatically reconciles that generation to every enrolled, authorized,
non-revoked Machine. Each credential bundle is sealed to the target Machine's
enrollment key, stored in a private Service-managed replica area, and exposed
only to the matching Provider runtime. Every online Machine acknowledges sealed
replica storage; a Machine with the Provider installed additionally acknowledges
typed materialization and its runtime probe. The Service's aggregate distribution
reports `converged` only after those applicable acknowledgements. Offline
Machines remain pending and reconcile automatically on reconnect. A newly
enrolled Machine receives the current generation without another login.

The Service owns auth state `signed_out`, `authenticating`, `ready`, `expired`,
or `error`, plus aggregate distribution `idle`, `distributing`, `converged`,
`degraded`, or `revoking`. A Machine may report only sealed-replica convergence
such as `pending`, `storing`, `current`, `failed`, or `revoking`, plus
materialization `not-installed`, `applying`, `current`, or `failed`. Those are
not login states. An offline Machine may degrade distribution without changing
the Service from authenticated `ready` or blocking a current online Machine.

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
Reinstall does not silently restore soft-deleted sessions.

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
Cowboy verifies publisher trust, digest, signature, archive safety, SBOM,
provenance, interfaces, capabilities, migrations, and probes again. Provider UI
cannot render pre-activation failures. Credentials, login codes, tokens, and
credential-bearing state never enter UI IR, logs, telemetry, Catalog metadata,
Provider artifacts, or ordinary Cowboy database rows.

## Minimum acceptance suite

The Provider platform is not complete until automated acceptance proves:

- each Provider builds from its own clean checkout without Cowboy source;
- malformed, mistyped, over-capable, unsigned, incompatible, and wrong-platform
  packages fail before activation;
- a release can install, upgrade, roll back, and uninstall independently on two
  Machines while existing sessions retain their exact generation;
- one Cowboy Service login automatically converges the same authentication
  replica to every connected Machine, materializes it on every installed
  Provider, and later converges it to an offline or newly enrolled Machine;
- concurrent credential refresh cannot fork the authoritative generation;
- Service logout blocks new sessions and causes every Machine replica to be
  wiped or marked pending revocation until it reconnects;
- uninstalling one Machine's Provider removes only its affected sessions and
  does not delete Service authentication, source projects, or worktrees; and
- Cowboy Web, Controller, and Machine candidates refuse activation when they
  would orphan an installed or session-leased Provider contract.

## Migration order

1. Publish the SDK, component library, package schema, auth contract, trusted
   verifier, conformance suite, and Catalog.
2. Add Service-owned authentication generations and automatic Machine replica
   reconciliation; retire Machine login UI only after end-to-end convergence.
3. Add persistent Machine-scoped Provider installations and exact session
   generation identity.
4. Convert one Provider and prove build, install, login synchronization,
   refresh, upgrade, rollback, uninstall, logout, and retention end to end.
5. Migrate the remaining Providers independently, then remove static Provider
   UI tables and user-visible internal component controls after final drain.
