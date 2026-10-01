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
- The complete `just check-compact` gate passed before integration, including
  19 isolated PostgreSQL tests and optimized executable builds.
- Repeated `just check-compact` after integrating `e982da42`: passed, including
  1,676 all-features Rust tests, 405 standalone Machine tests, 1,959 frontend
  tests, the isolated PostgreSQL gate and optimized builds.
- No physical iPhone or production private-repository authentication acceptance
  is claimed.

## Initial compatibility block (resolved by the rollout below)

The actual active Controller release
`/nix/store/90qr5ijalm9v6x0spwnjwfqx271jndik-cowboy-controller-release`
(source `dbd19bbf74e039d38962f27ff91eb2e8da4ddcfb`) rejected the new package
in a disposable Catalog with `unknown field review`. This was a diagnostic
using the candidate package, a temporary Ed25519 key, independently verified
fixture signature and an HTTPS fixture artifact URL. It changed no production
Catalog, policy, credentials or installation. It does **not** replace the owned
immutable reader-conformance publication gate.

Publishing into that old reader floor could break refresh or restart. The
candidate remained unpublished until compatible active, recovery and cold
readers were accepted. The subsequent rollout is recorded below.

## Built candidates

Clean integrated source `0bf0ca61e3fa945f0112faef9beb0635b659b33d` produced:

- Controller: `/nix/store/hq3ghhy1zijnhrk6kxvy3psx7183ya32-cowboy-controller-release`
- Machine: `/nix/store/djh60qw50afackpp2qsy0q6a5nhjnwdn-cowboy-machine-release`
- Web: `/nix/store/davmxm1af9x7sv0fhnllpjybhhyrajrl-cowboy-web-release`

The [reader diagnostic](remote-pr-review-reader-diagnostic-2026-10-01.json)
compares the actual active Controller with this immutable candidate using the
same verified temporary-signature package. The candidate reads GitHub 0.2.0;
the active reader refuses it. This proves neither the production recovery/cold
floor nor publication, installation or host activation acceptance.

## Production rollout completed

Hawk now runs the compatible Controller and resident Machine, and GitHub 0.2.0
is installed and active. The [closed rollout receipt](remote-pr-review-rollout-2026-10-01.json)
records immutable paths, actual component transactions, reader identities,
installation revision, public artifact digest, browser checks and HTTP checks.

- Controller transaction: `1790828724546704189-417697f69680`.
- Machine transaction: `1790829607523752090-9747cde340a9`;
  observed worker generation `worker-401a0858f41aa4ab4470`.
- Hawk cold recovery floor: Columbus `de7faa3b55e12770d23aa7ac23c5a1cdb8aec91a`,
  published to its remote main; owned activation transaction
  `1790829273075308097-de7faa3b55e1` succeeded with no new failed units.
- Retained the concurrently published Web source `c5d608f90db4`, transaction
  `1790829809189110697-c5d608f90db4`, service-worker cache `cowboy-v1778`.
  The served shell, service worker and entry asset match that immutable output.
  Shell and worker revalidation and immutable asset caching were checked.
- GitHub digest: `sha256:8eceeaa8091f7708dc03cfa6493f5e0b950f297b03bf76d10ac8061c7339998f`.
  Both Catalog roots contain the independently verified signed package;
  the primary Catalog advertises it as ready. The public HTTPS package matches
  its digest, checked through the public TLS authority with loopback origin
  resolution (not an external routing test).
- Added only GitHub to Hawk's membership declaration and converged only that
  Plugin. Operation `hawk-github-0-2-0-converge` completed with an Applied
  Machine receipt; installed revision
  `installation-6b778f740853aede5feba3b98f026e44bff6dcd4a5535e3506ae9e5b2382b308`.
  No other Machine membership or Agent installation was changed by this task.

All six shared-SDK Agent packages were signed, verified and published before
Controller activation; exact release coverage passed. Their private dependency
pins are unchanged. Every package passed actual macOS arm64 execution and Linux
candidate-worker initialize/session-new, descendant drain and distinct-generation
coexistence. These publications do not upgrade installed Agent Plugins. Zed
1.20.3 was not published or installed by this rollout.

Postactivation gates used actual active, next-transaction recovery and cold
outputs: 174 Controller installation-reader checks, 84 Machine reader checks,
dataset lifecycle reads, telemetry reads, background startup and writer policy.
The complete copied production Catalogs accepted exact GitHub 0.2.0 bytes in all
16 immutable-reader opens before publication. Actual Controller and Machine
configuration preflights also passed without exposing private configuration.

The final Firefox workspace-extension gate passed all 19 cases on source
`9747cde3`, including the six remote-review cases. Its fixture SHA-256 is
`1f3b4d3457e578d6337110be9f85cb3a87166bc33ccab11f629765c2d2292af2`.
Review Code and Diff browser regressions passed separately. Subsequent integrated
changes affect Provider sign-in and have their own acceptance receipts.

All 18 pre-existing worker PID/start identities survived Controller, cold-floor
and Machine activation. At the later final observation, one separate session
had exited and successfully resumed at 04:44:22 UTC; the other 17 were unchanged.
Its cause was not established, so this is not a claim of uninterrupted PID
continuity through the entire verification window.

The existing filtered GitHub CLI connection can read the selected repository;
that repository has no PRs, so production PR patch acceptance is not claimed.
Physical iPhone performance and a native iOS release remain unverified. Reload
the mobile PWA to pick up the Web release, open Review, associate a PR and switch
between local worktree and Remote PR. Review is read-only; commenting and
submitting a GitHub review are outside this implementation.
