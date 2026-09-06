# Pre-cutover Catalog reader bridge

This branch is a narrow backport onto deployed Cowboy revision
`c293e0e91d00744cf4d82034c1be789b30c5e44c`. It prepares that legacy Controller to
coexist with append-only publication of newer Plugin formats; it does not
upgrade a Provider, SDK, Machine protocol, authentication implementation, host
runtime, Web bundle or SQL migration.

The activation candidate also incorporates fresh `origin/main` revision
`4741d063b94084b13d6c3beab97a3a43371f55d3`, which changes only Web source/tests
and their documentation. Those changes do not alter the Controller backport;
this task builds and activates only the Controller release, not a Web release.

The reader inspects the release envelope before the package. No envelope means
an incomplete publication, not a candidate. An envelope with a future schema
is ignored without parsing its package or trusting its claimed ID/version. It
cannot mask a supported signed release, create an installation target, change
an authentication selection, or activate a host. Supported schema-1 releases
still pass every existing package, identity, digest and signature check. A bad
supported release aborts refresh while preserving the old snapshot, and fails
cold start. Malformed/ambiguous headers, symlinks, non-regular envelopes and
envelopes over 1 MiB are rejected rather than treated as future releases.

The follow-up reader candidate also handles hostless future Code payloads,
whose outer release schema can remain 1. Before decoding runtime component
variants it checks the exact package digest and explicit nested Code schema.
Only a payload newer than the supported Code schema 1 is skipped, without
trusting its identity. Both manifest and payload must have the Code kind and
the package must use the supported outer package schema. Missing, duplicate,
zero or non-integer discriminators fail closed. Packages must be regular,
non-linked files no larger than the existing Machine limit of 8 MiB. Supported
Code schema 1 retains all SDK and signature checks; arbitrary decode errors
never become a compatibility skip. This is a format rule, not a Plugin-ID
exception, and does not upgrade the SDK or activate a Code runtime.

This follow-up is a build/test candidate, not an activated reader floor. Its
publication gate must inspect each exact fully bound release using the actual
immutable reader and successor binaries. The older `1814cb19e152` reader cannot
decode Zed 1.2.1's Code schema 2 even though its outer release schema is 1.

Read-only inspection uses the same Catalog reader:

```sh
cowboy serve --check-plugin-catalog \
  --data-dir /absolute/service-data \
  --plugin-catalog-dir /absolute/catalog
```

It creates no state, opens no database/listener and runs no Plugin or login.
Its public JSON report lists only supported, verified exact release identities.
It advertises `supported_code_payload_schema: 1` so the conformance harness can
distinguish an explicit future-format exclusion from unexplained inventory loss.
It is not runtime, host-policy, migration or authentication acceptance.

This is **not** a rollback of activated Plugin hosts/storage. Both inspection
and normal startup reject `plugins/.catalog-only-v1` and any
`plugins/live/<id>/.catalog-authority-v1` before Service initialization. Restore
a host-capable Controller with its exact policy after those boundaries; never
delete markers or substitute this legacy reader to bypass them.

Before any production publication, both the active Controller and its actual
automatic rollback target must have accepted reader behavior. Merely building
or activating the bridge once does not replace an incompatible rollback
predecessor. Inspect the machine-owned activator's rollback receipt/floor; never
fake it by repointing a profile. Subsequent host activation still needs its own
accepted forward/recovery transition. Keep all old signed releases and their
immutable bytes in the Catalog while old consumers require them.

For Hawk's installed Columbus activator `a872284`, rollback restores the
`previousRelease` captured from the active profile under the machine lock for
that transaction only. After success is Git-pinned, receipted and the journal
removed, the next transaction captures the now-active bridge as its predecessor.
An older `previousRelease` in a completed receipt is historical evidence, not
an automatic post-success downgrade target. Verify this implementation and the
actual profile/receipt/journal together; do not infer the next rollback target
from the previous Nix generation number. Manual post-success rollback requires
a new descendant revert that preserves the applicable compatibility boundary.

From the repository root in its pinned shell:

```sh
nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --lib --locked plugin_catalog::tests
nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --lib --locked
nix build .#cowboy-controller-release --no-link
```

The Nix artifact must come from a clean committed worktree. Building, signing
temporary test fixtures, and reading a test Catalog do not authorize production
activation, publisher signing, Catalog publication or an authentication change.
