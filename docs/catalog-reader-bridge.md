# Pre-cutover Catalog reader bridge

This branch is a narrow backport onto deployed Cowboy revision
`c293e0e91d00744cf4d82034c1be789b30c5e44c`. It prepares that legacy Controller to
coexist with append-only publication of newer Plugin formats; it does not
upgrade a Provider, SDK, Machine protocol, authentication implementation, host
runtime, Web bundle or SQL migration.

The reader inspects the release envelope before the package. No envelope means
an incomplete publication, not a candidate. An envelope with a future schema
is ignored without parsing its package or trusting its claimed ID/version. It
cannot mask a supported signed release, create an installation target, change
an authentication selection, or activate a host. Supported schema-1 releases
still pass every existing package, identity, digest and signature check. A bad
supported release aborts refresh while preserving the old snapshot, and fails
cold start. Malformed/ambiguous headers, symlinks, non-regular envelopes and
envelopes over 1 MiB are rejected rather than treated as future releases.

Read-only inspection uses the same Catalog reader:

```sh
cowboy serve --check-plugin-catalog \
  --data-dir /absolute/service-data \
  --plugin-catalog-dir /absolute/catalog
```

It creates no state, opens no database/listener and runs no Plugin or login.
Its public JSON report lists only supported, verified exact release identities.
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

From the repository root in its pinned shell:

```sh
nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --lib --locked plugin_catalog::tests
nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --lib --locked
nix build .#cowboy-controller-release --no-link
```

The Nix artifact must come from a clean committed worktree. Building, signing
temporary test fixtures, and reading a test Catalog do not authorize production
activation, publisher signing, Catalog publication or an authentication change.
