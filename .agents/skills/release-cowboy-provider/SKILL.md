---
name: release-cowboy-provider
description: Audit and upgrade one or more Cowboy Provider packages' private pinned dependencies, run the owning conformance and compatibility gates, build and sign immutable artifacts, publish a new Provider version, and verify Catalog availability. Use when asked to check Provider dependency updates, upgrade a Provider's agent or internal ACP implementation, test a Provider candidate, release a Provider, or make a released Provider version available for per-Machine installation.
---

# Release Cowboy Provider

Release each Provider as an independent artifact. Keep its transport, agent
adapter, gateway, model client, and other implementation dependencies private
to the package; Cowboy users install only a named Provider version on a named
Machine.

Keep this skill canonical in the Cowboy repository at
`.agents/skills/release-cowboy-provider`. Update and review it with the Provider
contracts; never fork it into a user-home skill.

The six first-party Provider sources live under `providers/<id>/provider.json`:
`claude-code`, `codex`, `gemini`, `grok`, `claude-deepseek`, and
`codex-deepseek`. Each source builds independently even though this repository
owns their common SDK and release tooling.

Before changing a Provider, read
[`docs/requirements.md`](../../../docs/requirements.md) and
[`docs/provider-packages.md`](../../../docs/provider-packages.md) completely.
Then read the selected source manifest and the repository-owned build and
release entrypoints. An external Provider may own a separate checkout; in that
case resolve it and read its nearest `AGENTS.md`, manifest, lock, and commands.
Never invent a command that its repository does not own.

## Preserve the boundary

- Treat `(provider_id, provider_version, artifact_digest)` as the release unit
  and `(machine_id, provider_id)` as the installation slot.
- Keep ACP and equivalent transport or adapter details inside the Provider.
  Do not add them to ordinary Cowboy labels, settings, cards, install dialogs,
  or version selectors.
- Pin every executable and protocol implementation used by the Provider to an
  exact immutable version and digest. Reject ranges, moving tags, `latest`, and
  unversioned runtime downloads.
- Disable dependency-owned auto-updaters. A dependency change always produces
  a new Provider artifact and Provider version.
- Do not upgrade a Provider dependency through Cowboy core, host NixOS, or a
  Machine-wide package. A required core contract change is a separate task.
- Keep release and installation separate. Publishing makes a version available
  in the Catalog; it never installs or upgrades that version on a Machine.
- Keep Provider authentication Cowboy Service-scoped. A release must not add a
  Machine login control, perform login, or mutate the active Service credential.
- Keep lifecycle effects on their typed surfaces: Service login/logout on
  `setup`, Machine install on `empty`, and Machine upgrade/uninstall on
  `settings`. The exact installed package owns a Machine card's layout; a newer
  release is only an upgrade target until activation succeeds.
- Treat Machine credentials only as versioned replicas of the Service auth
  generation. Never let a Provider dependency create an independent Machine
  account, updater, refresh lineage, or credential store.

## Audit candidates

For first-party Providers, start with the read-only authoritative registry
audit from this skill directory:

```bash
deno run --allow-read --allow-net scripts/audit-dependencies.ts <provider-id>
```

Use `all` only for an audit report; still upgrade, test, version, and release
each Provider independently. Git-pinned private gateways are reported as
`manual_git_review` and require upstream source/release inspection rather than
guessing from a moving branch.

1. Capture the Provider ID, current Provider version and digest, supported
   platforms, public contract versions, authentication-contract fingerprint,
   and every private dependency pin.
2. Query authoritative upstream release and security sources for each pin.
   Record the newest candidate, release date, relevant changes, license or
   platform changes, and whether the current pin is already preferred.
3. Classify each candidate as `safe-to-test`, `blocked`, or `no-upgrade`.
   Do not infer compatibility from SemVer alone.
4. Produce an upgrade plan before mutation. Name the exact files, expected
   Provider version bump, tests, and target platforms.
5. When auditing all Providers, report and release each Provider independently.
   First-party sources may share this repository and commit, but they never
   share a Provider version, artifact, signature, or Catalog transaction.

## Upgrade one Provider

1. Update only the selected Provider's private dependency declarations and
   lock data. Preserve unrelated changes.
2. Regenerate derived locks or bindings using the Provider repository's
   commands. If that repository owns SBOM or provenance generation, regenerate
   those too; do not claim Cowboy schema v2 requires metadata it does not parse.
3. Verify that the built package uses the requested exact pins and performs no
   runtime dependency resolution or self-update.
4. Inspect all ordinary user-facing surfaces. Present the Provider name and
   Provider version; reject accidental ACP, adapter, gateway, or internal
   dependency controls. Reject Machine-specific login, logout, account, or
   credential controls. Developer-only diagnostics may identify internals or
   auth-replica convergence only when the package contract explicitly marks
   that surface as diagnostic.
5. Bump the Provider version. Use a patch for a dependency-only compatible
   release, and a minor or major when the public Provider contract requires it.
   Never republish different bytes under an existing version.

## Verify the candidate

Run the Provider's complete deterministic gate inside its documented toolchain.
In addition, require all applicable Provider gates below:

- Validate package schema 2 and release schema 2; recompute requirements from
  the actual UI IR, runtime contract, authentication contract, and artifact
  matrix.
- Run the trusted Cowboy UI IR type checker; reject invalid component props,
  message payloads, reducers, state transitions, effects, capability use, or
  resource bounds. Require Rust package validation and TypeScript Catalog
  validation to agree on lifecycle-effect surface ownership.
- For UI schema 2, validate responsive wrapping, bounded vector gradients, and
  the closed activity indicator/label unions in both implementations. Reject
  unknown strategy fields and any Provider-ID branch in the host renderer.
- For SDK 2.3 host integration schema 2, validate the required Transcript
  presentation contract in both implementations. Accept only `timeline`,
  `workcell`, `signal`, or `terminal` with bounded density, active-label, and
  current-surface tokens. Reject host-schema downgrade smuggling, unknown
  fields, arbitrary style values, and any Provider-ID branch in the Transcript
  renderer.
- Validate signed `configuration.options` policy. Provider-specific option
  ordering, layout, and lifecycle availability belong in the package; reject
  a Cowboy Web branch on a Provider-specific configuration option ID.
- Validate signed `host.tool_presentations` links and their closed renderer
  IDs. Reject Provider-ID/tool-name dispatch tables in Cowboy Web.
- Validate every package-selected behavior profile and runtime command against
  the closed SDK contract. Exercise the real initialize and session lifecycle
  when the Provider owns a hermetic conformance harness.
- For SDK 2.1+ runtime graphs, validate every `component_command` and
  `sidecar_url` link on every platform. Gateway behavior, capability, private
  component, session sidecar, auth-environment forwarding, and readiness must
  agree exactly. Reject fixed Machine-global ports, host profile/resource
  paths, shell templates, or ambient CLI/adapter/gateway fallbacks in an exact
  package.
- Exercise each declared sidecar with its released component: allocate a fresh
  loopback port, start it with the declared listen argument, require the exact
  health path within the timeout, launch the ACP entrypoint with resolved
  bindings, and prove worker teardown reaps the sidecar. Run old/new generation
  sessions concurrently when a Provider has sidecars so upgrade drain cannot
  be satisfied by sharing replacement runtime bytes.
- Run unit, integration, failure-path, authentication-state, and upgrade tests.
  Never put real credentials in fixtures or logs.
- Run applicable typed Service-authentication tests with hermetic bundles.
  Cover generation compare-and-swap, per-Machine sealing/materialization,
  exact-release temporary-executor and candidate binding, logout wipe, and the
  invariant that uninstall does not log out the Service.
- Prove every declared OS and architecture has a matching artifact. Test each
  supported target or use the repository's accepted cross-platform evidence.
- Build from a clean committed source and verify the package contains the
  expected exact dependency pins, runtime command links, contract fingerprints,
  and no undeclared fields.
- Run the compatibility check against the minimum supported and current Cowboy
  Web, Controller, and Machine contract inventories. A live ACP handshake is
  behavior evidence, not a substitute for artifact interface validation.

Do not release when any deterministic gate, platform target, interface check,
runtime binding, or required authenticated smoke test is unresolved.

For a first-party Provider, the minimum package gate is:

```bash
nix develop -c just provider-check
nix develop -c just check
```

`just provider-build <provider-id>` is the independent data-only package build.
It writes `dist/providers/<provider-id>/<provider-id>.cowboy-provider` plus an
unsigned, deliberately unbound release envelope. `provider-check` checks the
typed runtime lock and npm lock payloads and builds all six separately so a
shared SDK change cannot leave one Provider un-linkable. An unbound envelope
may be reviewed in Web but is never installable.

## Publish and verify

Publish only when the user requested a release and the Provider repository's
release authority permits it.

1. Commit the complete Provider change according to its repository policy.
2. Build the final data-only package and every declared runtime target from
   that exact clean commit:

   ```bash
   nix develop -c just provider-release-build <provider-id>
   ```

   This uses `providers/runtime-lock.json` and the isolated npm v3 lock payloads
   under `providers/runtime-packages/`, probes supported host artifacts, assigns
   content-addressed HTTPS URLs, and binds the runtime-artifact matrix. A
   gateway probe must terminate without credentials; use its owned help/version
   mode, not the long-running listen command. Binding must reject a missing or
   extra target, missing or extra component, wrong
   kind/slot/dependency/version/command, mutable URL, unsafe entrypoint, invalid
   digest, or invalid probe. It computes the composite `artifact_digest` over
   the package and full runtime matrix.
3. Sign the complete release with the configured Ed25519 Provider publisher
   identity, then verify it with the independently selected public key.
4. Publish the package, adjacent signed release envelope, trusted public key,
   runtime bytes, and receipt through the Provider-owned release command. The
   command must independently verify the signature before writing Catalog
   bytes:

   ```bash
   nix develop -c just provider-publish <provider-id> <catalog-directory> <publisher-public-key>
   ```

   Publish repository-owned SBOM/provenance alongside them when available, but
   do not put them in the v2 trust claim.
5. Resolve every published package and runtime URL back to immutable bytes and
   require its digest to match the bound manifest.
6. Call the Catalog refresh endpoint and verify that Cowboy advertises that
   exact version and digest, including its supported Machine platforms and
   compatibility report.
7. Stop. Report that the version is available for UI installation; do not call
   a Machine installation or upgrade endpoint unless the user separately asks
   to install it on a specific Machine.

For first-party artifacts, sign and independently re-verify the exact output:

```bash
nix develop -c just provider-sign <provider-id> <publisher-private-key>
nix develop -c just provider-verify <provider-id> <publisher-public-key>
```

Copy the artifact and adjacent `.release.json` into the configured external
Catalog together with the publisher public key under
`trusted-publishers/<publisher>.pub`, then call the Catalog refresh endpoint.
The refresh must return success and `/api/providers` must advertise the exact
version, package digest, composite artifact digest, contract fingerprint,
`release_state=ready`, and platform matrix before the release is reported as
available. An embedded `release_state=unbound` entry is not a release receipt.

Do not perform or refresh a Cowboy Service login as release verification. Use
hermetic auth fixtures unless the Provider's repository gate explicitly
requires an authorized smoke test, and never publish the resulting credential
state.

Return an upgrade and release receipt containing the Provider ID, old and new
Provider versions, old and new dependency pins, source commit, artifact digest,
signature identity, contract fingerprints, platform matrix, gates run, Catalog
observation, authentication-contract fingerprint and conformance result, and any
blocked Machine targets.

## Fail closed

If the selected Provider lacks a package manifest, exact private pins, typed
Service authentication contract, trusted verifier, repository-owned quality
gate, signing identity, or Catalog publication target, report the missing
prerequisite. Do not simulate a Provider release by editing the legacy
`LaunchSpec` fallback, changing an unpinned runtime launch, adding per-Machine
login, or publishing a Cowboy Controller or Machine release.
