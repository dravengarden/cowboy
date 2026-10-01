# Remote PR review implementation — 2026-10-01

The initial implementation adds session-associated, read-only remote PR files
and patches to mobile Review. See [the feature contract](../remote-pr-review.md).
It requires GitHub 0.2.0, workspace payload 2 and Plugin SDK 1.10.0. Component
release 3.32.0 propagates the exact shared SDK/contract dependency pins; no
Agent private dependency was upgraded.

## Validation

- Plugin component/package validation passed for all eight source Plugins.
- Rust all-features tests: 1,667 passed; standalone Machine: 399 passed;
  standalone Code adapter: 31 passed. Ignored conformance tests are separate
  acceptance gates, not counted as passed.
- Frontend tests: 1,957 passed.
- Firefox 151.0.1 workspace-extension browser gate: 19 passed, including six
  remote-review cases using the actual CodeMirror renderer. Final fixture
  SHA-256: `b96d6442e080a739d19a1f7ae144407a3d11d48c2e66fa6bc673727d60fec807`.
- Plugin pack 1.10.0 built through the immutable Nix output after refreshing
  the Cargo vendor hash.
- No physical iPhone or production private-repository authentication acceptance
  is claimed.

## Production rollout blocked; nothing published or activated

The actual active Controller release
`/nix/store/90qr5ijalm9v6x0spwnjwfqx271jndik-cowboy-controller-release`
(source `dbd19bbf74e039d38962f27ff91eb2e8da4ddcfb`) rejected the new package
in a disposable Catalog with `unknown field review`. This was a diagnostic
using the candidate package, a temporary Ed25519 key, independently verified
fixture signature and an HTTPS fixture artifact URL. It changed no production
Catalog, policy, credentials or installation. It does **not** replace the owned
immutable reader-conformance publication gate.

Publishing the new schema into the current Catalog could break refresh or
restart. Keep it unpublished until compatible active, recovery and cold readers
are accepted. Then complete exact package signing/publication and shared Agent
release coverage before Controller activation. Machine reader activation and
installing GitHub 0.2.0 remain explicitly separate maintenance/installation
steps; do not restart resident Machine or existing sessions as a Web release.

No Controller, Web, Machine, Plugin or iOS release is claimed by this ledger.
