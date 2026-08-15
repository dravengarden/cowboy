# Installable Provider packages

Status: Provider package schema v2, release schema v2, and host integration
schema v2 implementation contract. The in-tree
`LaunchSpec` registry remains only as a compatibility fallback for pre-package
session generations.

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
slot. `package_digest` covers the canonical data-only `.cowboy-provider` file;
`artifact_digest` is the composite identity of that package and every declared
platform runtime artifact. The same release may be installed on Hawk, Falcon,
both, or neither.

Cowboy's ordinary UI exposes the Provider name, artwork, Provider version,
capabilities, Machine installation health, the Cowboy Service's single
Provider-authentication state, and Provider-owned surfaces. Authentication is
not nested under a Machine. The UI does not expose ACP, an adapter package, a
gateway, a model wire protocol, or any other transport implementation as a
separately selectable product concept. Developer diagnostics may reveal those
facts only through an explicitly scoped diagnostic surface.

`claude-code`, `codex`, `gemini`, `grok`, `claude-deepseek`, and
`codex-deepseek` are each independent Provider packages and installation units.
Private runtime artifacts are staged inside the owning Provider generation and
are never exposed as installable subcomponents. Shared-blob deduplication may be
added later without changing the Provider installation boundary.

## Package ownership

Each Provider builds independently against a published Cowboy Provider SDK and
contract bundle. The first-party sources are co-located under `providers/`, but
each has its own source, version, artifact, release envelope, signature, and
install transaction. Its artifact owns:

- its Provider descriptor and typed UI surface IR;
- logo, icon, loading, card, settings, information, empty, and error surfaces;
- its typed runtime and agent-launch contract;
- exact internal executable and protocol dependency pins;
- one immutable runtime-artifact binding per declared platform and private
  component;
- its typed Service login, credential, Machine projection, refresh-ownership,
  revocation, and wipe contract;
- contract requirements and fingerprints;
- publisher identity and release signature.

The package may use ACP internally, but that dependency is pinned inside the
Provider. A mutable command such as `npx -y <unversioned-package>`, a moving
container tag, a dependency-owned auto-updater, or a runtime download is not a
valid released Provider. An internal dependency upgrade always produces new
artifact bytes and therefore a new Provider version.

Provider source repositories own their build and quality gates. The published
SDK supplies authoring types, the typed UI DSL, host bindings, the reference
verifier, conformance fixtures, and packaging tools. Cowboy repeats
all security- and compatibility-critical validation during installation; it
does not trust a Provider's build receipt.

### Exact runtime graph and session sidecars

Runtime values form another closed DSL. A Provider argument or non-secret
environment value is either a bounded literal, a `component_command` binding
to a declared kind/slot, or a `sidecar_url` binding to a declared sidecar. A
binding may add bounded prefix and suffix text for configuration formats, but
it cannot invoke a shell, read ambient state, or name an undeclared resource.

A sidecar declares a stable ID, one `provider_gateway` component, literal
arguments and environment, an explicit subset of Service-auth environment
projections, and a `loopback_http_v1` transport with listen flag, health path,
and bounded timeout. Cowboy validates that every component and binding exists
on every advertised platform and that gateway capability, gateway behavior,
private gateway components, and sidecars agree exactly.

At session launch the Machine passes a verified map of all executable paths
inside the exact content-addressed generation and prepends only their parent
directories to that worker's `PATH`. The worker allocates a unique loopback
port, starts each sidecar, waits for readiness, resolves typed values, and then
starts the ACP entrypoint. Sidecar handles live as long as the worker. This
permits old and new Provider and authentication generations to run concurrently
without fixed-port replacement, and prevents a signed package from falling
back to a Machine-global CLI, adapter, gateway, or resource path.

## Typed UI SDK and component library

Cowboy publishes a versioned Provider UI component library as part of the
Provider SDK. It is a contract and authoring library, not a second copy of the
Cowboy application. Provider authors compose typed primitives and Cowboy-owned
shell slots for cards, setup, settings, information, empty, loading, error, and
session surfaces. Provider packages supply their own content-addressed logo,
icon, illustration, and loading assets through typed asset references. UI
schema v2 additionally supplies bounded vector gradients, responsive stack
wrapping, and a closed activity node.

Cowboy renders `information` and `setup` once in the Service authentication
region. A Machine card renders `card` plus `empty` before installation or
`settings` afterward; it never embeds login or logout. When a newer release is
available, the exact installed manifest continues to own that Machine's card
and settings layout, while the latest compatible signed release is only the
upgrade target. Cowboy keeps both Service authentication details and per-Machine
Provider lifecycle details collapsed behind compact summaries by default; the
Provider surfaces render inside the explicit management region.

Authentication copy and actions are also typed. Cowboy maps a Provider with
only `secret_input` auth methods to the closed `api_key` presentation; every
other current method graph maps to `account`. The component library renders
the corresponding missing/configured status, add/replace/clear or
sign-in/sign-out verbs, secret field, and modal instructions. The Provider does
not provide arbitrary action labels, CSS, or layout geometry, and Cowboy
contains no Provider-ID authentication branch. Unavailable actions are omitted
rather than shown as disabled clutter, while status, version, and the valid
action share a stable footer slot.

The activity node separates Provider-authored identity from Cowboy-owned motion
mechanics. Its indicator is one of `progress_ring`, `glyph_cycle`,
`terminal_prompt`, `asset_signal`, or `asset_pulse`; its label is typed text or
a bounded phrase cycle with a `none`, `fade`, or `shimmer` effect. Assets,
intervals, frames, phrases, colors, and accessible labels are validated. Cowboy
implements reduced motion, animation timing, layout, theme behavior, and ARIA
semantics without a Provider-ID branch. This restores distinct Claude, Codex,
Gemini, and Grok activity language without allowing executable Provider UI.

Transcript thought presentation is a sibling host contract rather than a
Provider-authored free-form surface because its content and lifecycle come from
the canonical conversation timeline. Host integration schema v2 requires
Transcript presentation schema v1. Its thought profile selects exactly one of
four Cowboy component-library variants:

| Variant | Cowboy-owned presentation | Intended use |
|---|---|---|
| `timeline` | Small status dots and connected steps | Long, conversational reasoning streams |
| `workcell` | Lightbulb steps, workcell header, and dual-accent live text | Coding-agent execution plans |
| `signal` | Provider mark as the status signal | Brand-led synthesis workflows |
| `terminal` | Prompt markers and compact terminal rhythm | Command-oriented agents |

Each profile additionally selects `compact` or `comfortable` density, an
optional bounded active label, and a `plain` or `soft` current-step surface.
Those are semantic tokens, not CSS values. Cowboy fixes the marker size,
alignment, typography, spacing scale, theme blending, motion, reduced-motion
fallback, and ARIA behavior. Unknown variants, fields, and tokens fail the
Provider build, Catalog ingestion, Web validator, and Machine installation.

The same exact manifest owns Provider-specific presentation of configuration
options advertised by the runtime. `configuration.options` declares a unique
option ID, bounded order, `standard` or `full_width` layout, and
`live_session` or `idle_or_stopped` availability. Cowboy supplies neutral
defaults for portable option concepts, but its Desktop and Mobile components
never name a Provider-specific option. This keeps linked controls data-driven
without allowing executable UI code.

`host.tool_presentations` applies the same rule to bespoke tool bodies. A
Provider may associate an exact upstream tool name with one closed,
Cowboy-owned renderer such as `todo_list_v1`. Unknown renderers, duplicate
names, and oversized or non-printable-ASCII names are rejected. The shared
normalized-kind and generic raw/result renderers remain the safe fallback.

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
schema. Assets declare an ID, role, media type, digest, accessible label, and
either a constrained vector path or inline bytes. Unknown components, fields,
events, tokens, asset roles, or layout constraints fail the Provider build and
fail again when Cowboy parses the package. Unknown or duplicate configuration
option and tool presentation records fail at the same boundaries.

UI and host integration schema versions are negotiated independently from
package schema. Provider SDK 2.4 accepts UI schemas 1-2 and host integration
schemas 1-2, rejects any newer-schema field when a manifest claims an older
schema, and rejects an unknown future schema. Existing UI v1 or host v1 packages
remain installable and render compact neutral fallbacks. For session-facing
icon, activity, and Transcript chrome only, an exact older-schema session may
adopt the latest compatible signed presentation for the same Provider; its
runtime, options, lifecycle logic, authentication contract, and generation
remain pinned to the exact package.

## Typed linked logic

Provider UI behavior is a typed state machine rather than embedded application
code. The DSL declares:

- a closed Provider UI state schema;
- a discriminated union of messages and their payload schemas;
- pure reducer assignments from literals, prior state, or typed message fields;
- typed host/state equality, `all`, `any`, and `not` conditions;
- effects addressed to named Provider-driver capabilities; and
- typed success and failure messages for each effect.

A button can emit only a message accepted by that surface. A reducer may assign
only fields of the declared type. Visibility, enabled, labels, and host values
are closed unions with component-specific types. Every message, reducer,
effect, state field, and asset reference must resolve. Effects cannot access
credentials, network, files, processes, or Cowboy state directly; the host
mediates a closed capability union and validates its request.

Lifecycle capabilities have fixed surface ownership. Service login and logout
belong only to `setup`, Machine installation only to `empty`, and Machine
upgrade and uninstall only to `settings`; external documentation may appear on
any surface. The Rust compiler and downloaded-package verifier, plus the
TypeScript Catalog validator, reject a linked effect that escapes this mapping.

For example, a Provider card may derive `canAuthenticate` from the typed Cowboy
Service auth state, dispatch `Authenticate { method }`, show a typed progress
surface, and reduce the Service driver's result into `ready` or `error`. A
Machine card may consume only typed replica-convergence and installation health;
it cannot dispatch a login message. The same message and state schemas are used
by the Rust compiler/validator and the TypeScript renderer, so the relationship
is checked instead of being encoded as unvalidated event names. Logic that
cannot be expressed by the current closed DSL requires a future SDK schema and
host implementation; UI schema v2 does not load Provider JavaScript,
WebAssembly, or another executable UI escape hatch.

Static authoring types cannot make an artifact downloaded months later safe by
themselves. The canonical IR, component schemas, and closed capability contracts
are therefore the runtime type boundary. Cowboy recomputes messages, state
transitions, effects, asset references, runtime links, and fingerprints from
the artifact instead of trusting its declared compatibility summary.

## Artifact and compatibility

Schema v2 separates a universal data-only package from executable delivery:

- `.cowboy-provider` contains the complete manifest and canonical contract
  fingerprint; `package_digest` covers its exact bytes.
- `.release.json` binds the package to a complete runtime-artifact matrix. Each
  target contains the exact private component kind, slot, dependency, version,
  logical command, immutable URL, SHA-256 digest, format, entrypoint, and probe.
- `artifact_digest` is recomputed from the package digest, release schema,
  Provider identity, contract fingerprint, target matrix, and every runtime
  binding. The Ed25519 release signature covers that composite identity and a
  fingerprint of the runtime matrix.

`provider-build` intentionally creates an unsigned, unbound release envelope.
Such a package may appear in the Catalog as `release_state=unbound` so its UI
can be reviewed, but Cowboy disables install and upgrade. For a production
release, `provider-release-build` builds the data-only package and every
declared target from `providers/runtime-lock.json`, assigns content-addressed
HTTPS URLs, and calls `provider-bind-runtime`. Binding requires exactly one
runtime entry for every declared OS/architecture and exactly one matching
artifact for every private component. Signing is rejected until that link is
complete. A platform is advertised only after it has accepted execution
evidence; adding a platform is a new immutable Provider release, not a Catalog
metadata edit.

The binding input is a JSON array. This single-target fragment illustrates the
shape; a real release must contain every target and component declared by its
package:

```json
[
  {
    "os": "linux",
    "architecture": "x86_64",
    "components": [
      {
        "kind": "provider_cli",
        "slot": "example",
        "dependency": "example-cli",
        "version": "1.2.3",
        "command": "example",
        "artifact_url": "https://releases.example.invalid/example/1.2.3/linux-x86_64.tar.gz",
        "artifact_digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "artifact_format": "tar_gz",
        "entrypoint": "bin/example",
        "probe": { "args": ["--version"], "timeout_ms": 10000 }
      }
    ]
  }
]
```

Package schema 2, UI schemas 1-2, host integration schemas 1-2, logic schema 1,
auth schema 1, Cowboy Provider SDK 2.4, Controller contract 2, Machine contract
4, and release schema 2 are independently checked.
The SDK version is a coarse authoring filter: Cowboy accepts only the same SDK
major and a Provider SDK version no newer than the validating host. A newer SDK
or a different major fails the Provider build, Catalog ingestion, Machine
installation, and Web manifest validation. Structural validation and recomputed
fingerprints remain authoritative even when that SemVer pre-filter passes.
Validation proves, among other things:

1. unknown fields and enum values are rejected;
2. all UI slots, assets, state/message/reducer/effect links, presets, host
   profiles, auth methods, credential paths, runtime commands, component
   bindings, sidecar links, and readiness contracts are valid;
3. exact dependency pins have unique IDs and no ranges or moving versions;
4. compatibility intervals include the running Controller and Machine contract;
5. the release/package identities, platform sets, component sets, versions,
   commands, composite digest, publisher key, and signature all agree.

The Controller repeats package and signature validation at Catalog ingestion.
The target Machine repeats it again, selects only its exact platform, downloads
runtime bytes over HTTPS (loopback HTTP is test-only), enforces size and archive
path limits, verifies each SHA-256 digest, runs every component probe and the
Provider launch probe, and changes the active link only after success. A
missing platform, mismatched interface, unsafe archive, failed signature, or
failed probe leaves the previous active generation unchanged. Service sign-in
state is orthogonal to artifact compatibility. The content-addressed Machine
cache retains the exact downloaded raw or archive bytes and its metadata binds
each logical command to the executable, stored artifact path, and signed
digest. Cache reuse and uninstall compensation re-hash those stored bytes
against the signed release matrix; a corrupt retained generation is never
activated.

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
monotonic `auth_generation`. The encrypted vault record is the single durable
authority for its redacted state, account label, schema fingerprints, generation,
and ciphertext. Neither the vault key nor Provider authentication rows enter the
ordinary Cowboy database; Machine inventory reports only replica convergence.

The Service starts that flow with the exact signed Provider version, composite
digest, and authentication-contract fingerprint selected by the UI. A connected
Machine may act as the temporary executor only when that exact release is
active. The exported candidate repeats the version, digest, fingerprint,
portable schema, and method; Cowboy rejects any upgrade race or identity drift
before committing a generation.

The Service automatically distributes the current generation to every enrolled,
authorized, non-revoked Machine. A bundle is sealed to that Machine's enrollment
public key and stored under a private Service-managed replica root. Every
Machine verifies and acknowledges the signed envelope and generation without
requiring the Provider to be installed. A Machine with the Provider installed
also validates the exact auth contract and projection schema, atomically
materializes the declared credential files and environment, and acknowledges
materialization. Neither receipt can become an independent account or login
state. Replica generations are monotonic and immutable: stale generations and
same-generation envelopes with different meaning are rejected. Sessions open
the exact immutable auth projection generation recorded at creation rather
than following a mutable `current` link.

Online Machines reconcile as part of login. Offline Machines retain `pending`
convergence and reconcile immediately after reconnect. New Machines receive the
current generation after enrollment. Installing a Provider on a Machine that
already has the sealed replica automatically materializes it; no second login
is shown.

The Service owns auth `signed_out`, `authenticating`, `ready`, `expired`, or
`error`, plus aggregate distribution `none`, `pending`, `current`, `partial`,
`failed`, or `revoking`. Machine diagnostics separate replica `pending`,
`storing`, `current`, `failed`, or `revoking` from materialization
`not-installed`, `applying`, `current`, or `failed`. An offline Machine degrades
distribution without changing the Service's authenticated state. A Provider
that requires auth is schedulable on a Machine only when Service auth is ready
and that Machine has materialized the current generation.

`authenticating` and a pre-commit `error` are Service-wide transient state. A
first login reports generation zero until the credential bundle is validated
and durably committed; that transient entry is visible in the Catalog but is
never sealed to a Machine. Replica synchronization and session scheduling read
only the durable encrypted vault state.

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

The UI submits only an immutable Catalog version and composite digest. It never
supplies a Machine download URL or runtime component. Installation keeps all
staging private until validation succeeds:

```text
Catalog ready -> package/signature/interface verified -> target selected
              -> current auth envelope decoded and contract-checked
              -> runtime staged and probed -> active/auth links committed
```

Failure before `active` leaves the prior generation unchanged. A Provider
upgrade installs side by side; existing sessions stay pinned to their original
Provider generation, while new sessions use the newly active generation.
Activation always links the authentication projection contract. When a current
Service auth generation exists, staging also materializes it; a signed-out
Service does not block installation. Scheduling waits for the current
generation's materialization acknowledgement when auth is required. If auth
commit fails after the runtime link changes, the Machine restores the exact
previous active and rollback links before returning the failure.

Provider install, uninstall, auth-replica, and auth-candidate commands require
Machine protocol 3. The retained-generation reactivation command used for
uninstall compensation requires protocol 4; an older peer rejects it before a
destructive uninstall begins.

## Sessions and uninstall

Every session persists `provider_id`, `provider_version`, and
`provider_generation_digest`. Provider name matching is insufficient because
multiple generations may coexist.

Uninstalling a Provider from one Machine affects only sessions pinned to that
Machine and Provider installation. The confirmation plan includes the exact
session set, active-turn count, retained data classes, and absolute purge time.
Execution first fences new matching sessions and rechecks the exact generation
and session set. It explicitly cancels confirmed active workers, removes the
Machine's active Provider link and credential projection, then soft-deletes the
same session set and removes it from ordinary UI. Install and uninstall are
serialized per `(machine_id, provider_id)`, and prompts cannot cross an
uninstall fence. If the Machine command or database soft-delete transaction
fails after removal started, the Controller asks protocol-4 Machines to
re-verify and reactivate the exact retained signed generation, then restores
the previously live sessions. A failed step reports the original error plus
any compensation failure; a successful uninstall keeps the fence until a later
install. Session records use an absolute `purge_after_at` so later policy
changes cannot move an already confirmed deadline.

Uninstall wipes that Machine's materialized Provider credential state. Its
Service-managed sealed replica remains synchronized but
cannot be opened by an absent Provider; this lets reinstall consume the current
generation without another login. Only Service logout, Machine revocation, or
credential-generation retirement wipes the sealed replica. Uninstall does not
log the Cowboy Service out or affect another Machine's replica.

The confirmation modal names the Machine and Provider, separates idle sessions
from turns that will be cancelled, states that source projects and user
worktrees remain untouched, and shows local and ISO forms of the permanent
purge deadline. Active turns require a second explicit checkbox. Reinstalling
the Provider does not silently undelete sessions.

Project source and session worktrees are not Provider package data and are not
deleted. After the absolute deadline Cowboy hard-deletes session rows and
cascaded events; unreferenced content-addressed event attachments are reclaimed
when unreferenced, subject to a 24-hour minimum-age race guard. Inactive Provider
generation bytes are a Machine cache, not retained session data.

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

## Implemented package path and legacy drain

`crates/cowboy-provider-sdk` owns canonical package construction, schema and
fingerprint validation, runtime binding, release envelopes, Ed25519 signing,
verification, and the closed Rust contract. `packages/provider-ui-sdk` provides
the matching strict TypeScript component/logic contract and Cowboy renderer.
`just provider-build <id>` builds one unbound package; `just provider-check`
validates the typed runtime lock, npm lock payloads, package schemas, and all six
independent manifests. `just provider-release-build <id>` builds and probes the
complete declared runtime matrix, sets immutable content-addressed URLs, and
binds the release. A release then requires `provider-sign` and
`provider-publish`; publication independently re-runs `provider-verify` with
the supplied public key before writing any Catalog bytes.

`providers/runtime-lock.json` is the single release-input lock for embedded
Node.js distributions, target-specific native npm archives, and exact Git
gateway commits. Node-based components have isolated package roots and npm v3
lockfiles under `providers/runtime-packages/`; the lock checker requires the
direct package version, resolved URL, and SRI to match the Provider manifest.
The Machine never falls back to a global Node, npm, ACP adapter, Provider CLI,
or gateway.

The Controller Catalog compiles the six first-party source manifests as typed
`unbound` entries and loads installable releases only from its trusted external
Catalog directory. The default is `<controller-data-dir>/provider-catalog` and
may be overridden explicitly. `provider-publish` installs publisher public keys,
Catalog package/release pairs, receipts, and immutable bytes below
`artifacts/<sha256>/<filename>`. The Controller serves only those confined
content-addressed files at `/provider-artifacts/<sha256>/<filename>`, with an
immutable cache policy and digest ETag. This prevents a Controller-local,
platform-less component
inventory from being mistaken for a multi-platform Provider release. Web uses
only Catalog manifests for discovery, marks, card layout, setup, settings,
loading, empty, error, and session-facing identity. Service authentication is
rendered once outside Machine cards; default summaries keep the mobile surface
compact. A Machine renders its exact installed manifest and uses the latest
ready release only as the install or upgrade target. Exact older-schema sessions
may use the latest compatible same-Provider signed presentation for generic
session chrome without changing their pinned executable generation.
Install/upgrade effects are disabled for unbound entries. A target Machine
verifies the exact signed release again, stages its own target artifacts beside
the active generation,
checks private component links, probes every executable, and atomically
activates it with a rollback link. Runtime cache metadata schema 2 retains and
re-hashes the signed artifact bytes before cache reuse or compensation.
An exact worker then binds every staged command, owns any declared gateway as a
dynamic-port session sidecar, and refuses a corrupt, missing, dangling, or
Machine-global substitute instead of entering the legacy launch registry.

Service-owned encrypted authentication generations, transient login status,
and Machine-sealed replicas are orthogonal to installation. New sessions record
exact Provider version,
artifact digest, and auth generation. Uninstall plans snapshot the exact active
generation and affected sessions, fence new work, require explicit active-turn
confirmation, soft-delete the sessions, and retain their database data only
until the absolute deadline shown by Web. Cascaded event attachments are
reference-scanned and pruned when unreferenced, subject to a 24-hour minimum-age
race guard. Per-Provider lifecycle fences serialize install and uninstall, and
protocol-4 reactivation compensates a returned Machine or database failure by
restoring the exact retained generation and previously live sessions.

`src/provider/mod.rs` retains the historical `LaunchSpec` registry solely for
restoring sessions that predate exact package identity and for a bounded local
compatibility path. It is not a release surface, Catalog, or ordinary UI. New
Machine-backed sessions resolve their launch entirely from the installed signed
Provider generation. Delete the fallback after the last legacy generation has
drained.
