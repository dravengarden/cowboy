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

The future-envelope skip is not a general payload-compatibility mechanism.
For example, hostless Zed 1.2.1 has release schema 1 but Code payload schema 2,
with an owned adapter/server graph and new runtime component variants. A legacy
reader still attempts to decode that supported outer envelope and its package.
Do not infer publication safety from its schema number or from a different
Plugin's successful schema-2 fixture.

The separate `cowboy/catalog-reader-bridge-sess-1788279284752` branch now prepares
a follow-up reader candidate for that nested Code boundary. It remains a
pre-Host backport, not the full Controller or an activated production floor.
After validating the exact envelope/package digest, it inspects only the
explicit Code format headers. Outer package schema 1, matching manifest/payload
Code kinds, and a positive integer Code schema newer than 1 are required for
exclusion. Duplicate or malformed discriminators fail closed; packages must
be regular, non-linked files within the existing Machine 8 MiB limit. Supported
Code schema 1 still requires the complete old SDK and signature checks.
Unknown variants and arbitrary parse failures never become compatibility skips,
and an excluded future identity cannot replace any supported signed release.

That candidate advertises `supported_code_payload_schema: 1` in its read-only
report. The owned publication gate accepts `skipped_future_code_payload` only
when the exact package has first passed the successor SDK verifier, its nested
Code schema exceeds the reader's explicit limit, and both cold reads retain
exactly the independent signed legacy identity. The successor must still read
and validate the candidate, including its Host preflight. An older reader with
no advertised nested limit, an unexplained missing identity, or a nonzero exit
remains a failure. This changes no release schema, signed package or SDK pin.

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

For Hawk's installed Columbus activator `a872284`, rollback restores the
`previousRelease` captured from the active profile under the machine lock for
that transaction only. After success is Git-pinned, receipted and the journal
removed, the next transaction captures the now-active bridge as its predecessor.
An older `previousRelease` in a completed receipt is historical evidence, not
an automatic post-success downgrade target. Verify this implementation and the
actual profile/receipt/journal together; do not infer the next rollback target
from the previous Nix generation number. Manual post-success rollback requires
a new descendant revert that preserves the applicable compatibility boundary.

The owned `just catalog-reader-conformance` recipe compares actual immutable
bridge, baseline and successor releases. Add `--publication <bound-release.json>`
for each actual candidate after setting its final artifact URL and binding its
complete runtime matrix. These optional checks keep every proof field and
package/host byte intact, sign only temporary copies, and record each reader's
exact inventory. Bound hosts use a temporary exact Host policy pin, including
storage hosts that cannot acquire migration authority from Catalog defaults.
Choose a public legacy fixture with a different publisher
from the candidates to preserve its real signature and trust key. Incompatible
candidates produce a failing receipt and nonzero exit; safely skipped future
envelopes or Code payloads remain unavailable to the legacy reader. No production
signing key is used. This is neither runtime acceptance nor a production publication or
full Catalog/Host-policy test. Review the receipt's `not_checked` fields and
apply the canonical release skill's remaining gates before publishing.

From the repository root in its pinned shell:

```sh
nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --lib --locked plugin_catalog::tests
nix develop -c env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --lib --locked
nix build .#cowboy-controller-release --no-link
```

The Nix artifact must come from a clean committed worktree. Building, signing
temporary test fixtures, and reading a test Catalog do not authorize production
activation, publisher signing, Catalog publication or an authentication change.
