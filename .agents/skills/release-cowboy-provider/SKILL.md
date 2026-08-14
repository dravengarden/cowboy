---
name: release-cowboy-provider
description: Audit and upgrade one or more Cowboy Provider packages' private pinned dependencies, run the owning conformance and compatibility gates, build and sign immutable artifacts, publish a new Provider version, and verify Catalog availability. Use when asked to check Provider dependency updates, upgrade a Provider's agent or internal ACP implementation, test a Provider candidate, release a Provider, or make a released Provider version available for per-Machine installation.
---

# Release Cowboy Provider

Release each Provider as an independent artifact. Keep its transport, agent
adapter, gateway, model client, and other implementation dependencies private
to the package; Cowboy users install only a named Provider version on a named
Machine.

Before changing a Provider, read
[`docs/provider-packages.md`](../../../docs/provider-packages.md) completely.
Then resolve the Provider's source checkout and read its nearest `AGENTS.md`,
manifest, lock, and repository-owned build and release entrypoints. Never invent
a command that the Provider repository does not own.

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

## Audit candidates

1. Capture the Provider ID, current Provider version and digest, supported
   platforms, public contract versions, and every private dependency pin.
2. Query authoritative upstream release and security sources for each pin.
   Record the newest candidate, release date, relevant changes, license or
   platform changes, and whether the current pin is already preferred.
3. Classify each candidate as `safe-to-test`, `blocked`, or `no-upgrade`.
   Do not infer compatibility from SemVer alone.
4. Produce an upgrade plan before mutation. Name the exact files, expected
   Provider version bump, tests, target platforms, and any state migration.
5. When auditing all Providers, isolate each Provider in its own checkout and
   report it independently. Never create an aggregate Provider version or one
   cross-Provider release commit.

## Upgrade one Provider

1. Update only the selected Provider's private dependency declarations and
   lock data. Preserve unrelated changes.
2. Regenerate derived locks, SBOM, provenance inputs, or generated bindings
   using the Provider repository's commands.
3. Verify that the built package uses the requested exact pins and performs no
   runtime dependency resolution or self-update.
4. Inspect all ordinary user-facing surfaces. Present the Provider name and
   Provider version; reject accidental ACP, adapter, gateway, or internal
   dependency controls. Developer-only diagnostics may identify internals only
   when the package contract explicitly marks that surface as diagnostic.
5. Bump the Provider version. Use a patch for a dependency-only compatible
   release, and a minor or major when the public Provider contract requires it.
   Never republish different bytes under an existing version.

## Verify the candidate

Run the Provider's complete deterministic gate inside its documented toolchain.
In addition, require all applicable Provider gates below:

- Validate the package schema and recompute its declared requirements from the
  actual UI IR and driver artifact.
- Run the trusted Cowboy UI IR type checker; reject invalid component props,
  message payloads, reducers, state transitions, effects, capability use, or
  resource bounds.
- Validate and link the internal driver against every supported Cowboy host
  contract. Exercise the real initialize and session lifecycle through the
  Provider's conformance harness without exposing the transport in the UI.
- Run unit, integration, failure-path, authentication-state, and upgrade tests.
  Never put real credentials in fixtures or logs.
- Prove every declared OS and architecture has a matching artifact. Test each
  supported target or use the repository's accepted cross-platform evidence.
- Dry-run every required Provider-state migration from each supported installed
  schema version.
- Build from a clean committed source and verify the package contains the
  expected exact dependency pins, contract fingerprints, SBOM, provenance, and
  no undeclared files.
- Run the compatibility check against the minimum supported and current Cowboy
  Web, Controller, and Machine contract inventories. A live ACP handshake is
  behavior evidence, not a substitute for artifact interface validation.

Do not release when any deterministic gate, platform target, migration path,
interface check, or required authenticated smoke test is unresolved.

## Publish and verify

Publish only when the user requested a release and the Provider repository's
release authority permits it.

1. Commit the complete Provider change according to its repository policy.
2. Build the final immutable artifact from that exact clean commit.
3. Sign the artifact digest with the configured Provider publisher identity.
4. Publish the version, platform descriptors, SBOM, provenance, and signature
   through the Provider-owned release command.
5. Resolve the published reference back to an immutable digest and require it
   to match the locally verified artifact.
6. Verify the Cowboy Provider Catalog advertises that exact version and digest,
   including its supported Machine platforms and compatibility report.
7. Stop. Report that the version is available for UI installation; do not call
   a Machine installation or upgrade endpoint unless the user separately asks
   to install it on a specific Machine.

Return an upgrade and release receipt containing the Provider ID, old and new
Provider versions, old and new dependency pins, source commit, artifact digest,
signature identity, contract fingerprints, platform matrix, gates run, Catalog
observation, and any blocked Machine targets.

## Fail closed during migration

The current Cowboy in-tree `LaunchSpec` registry predates installable Provider
packages. If the selected Provider does not yet have a package manifest, exact
private lock, trusted verifier, repository-owned quality gate, or publish
command, report the missing prerequisite. Do not simulate a Provider release by
editing `src/provider/mod.rs`, changing an unpinned `npx` launch, or publishing a
Cowboy Controller or Machine release.
