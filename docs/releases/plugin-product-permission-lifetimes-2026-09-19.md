# Core product permission lifetimes — 2026-09-19

Published and activated the Controller-only
[product permission lifetime](../plugin-product-permission-lifetimes.md).
Verified product requests retain a private core observation of their original
effective role. Actual core policy mutations end affected observations before
unlocking. Owner → Viewer → Owner cannot revive an old request without an
intermediate poll; promotion cannot expand a queued request. Fresh requests
observe the new role. Unrelated users/settings and unchanged effective roles
retain their existing observations.

The same boundary covers product-backed Code readers and Operator installation,
uninstall, telemetry export/binding and recovery/resolution continuations.
Original credentials, enabled user, Session visibility, purpose and absolute
operation budgets remain independently required. Observed native effects are
recorded before response authority is checked; refusal does not discard a
result, replay a command or imply undo. Current and ended generations both
consume the bounded observation budget until their actual holders drop.

## Exact release

- Clean source and connected harness: `576594979141d7fc8171aa1a2087d9a15902b583`.
- Controller: `/nix/store/5ikk6lz3b44aryb8b5j4412iczlzg8km-cowboy-controller-release`.
- Executable: `/nix/store/4gw2sp2ybj0b6c54gx084fnhgk79h0d6-cowboy-0.1.0/bin/cowboy`;
  SHA-256 `8f9027973368db5ebe0b52acd75e606bae1de4f8544e8c3e6f0c544599afa2bc`.
- Previous Controller: `/nix/store/pqn8jrmmxv75z9f8ylss7g5zhj5r5pqs-cowboy-controller-release`,
  source `34ade0039fce3fae1a95ff902cd913d7743c1165`.
- Activation `1789817803553317169-576594979141` committed at
  `2026-09-19T11:37:07.717931947Z`: published, succeeded, non-maintenance,
  no recovery. Source was pushed to remote main before activation.

No Plugin/SDK, Machine protocol or native binary, SQL baseline, host policy,
production role, credential or Web release changed. The single-user permission
mutation API remains closed. Subsequent evidence/documentation commits do not
change the accepted runtime artifact.

## Gates

- Complete pinned-shell `just check-compact` passed: 1,555 main Rust tests
  (34 explicitly ignored), 375 standalone Machine tests (4 ignored), 26 core
  adapter tests, 126 private adapter tests (2 ignored), 1,832 Web tests and
  all 18 separately isolated PostgreSQL tests. Formatting, lint, types,
  dependency, feature/build gates and all 86 structural-link vectors passed.
- New source regressions exercise actual Hub policy mutations without polling,
  queued cookie/PAT authentication, promotion, unrelated-user and unchanged-role
  continuity, all settings mutation paths, unwind, foreign-core rejection,
  1,024-observation saturation and retention of ended generations until drop.
  Operator purpose matrices add no-poll role ABA. Parked successful/conditional/
  error Code replies are redacted; actual Open/Query/Release outcomes remain
  recorded and require fresh original-user authority to observe.
- Source-level negative: adding the role-ABA regression to the pre-fix
  `878d5ffe` sources fails at `captured.current(...).is_none()` after
  Owner → Viewer → Owner, in 0.02 seconds after compilation. This is not an
  old immutable Controller HTTP role-mutation test. Acceptance neither opens
  the closed permission API nor changes a production policy/database.
- Clean Nix artifact build passed 1,171 default-feature tests (18 ignored)
  and three bridge tests.
- Isolated Firefox `153.0.1` passed all 24 buffer-owner cases, fixture
  SHA-256 `4e7a11c5ac7d212fa57370ad912bca000d784143febfa4d683eece9bd7b2da27`.
  This is a fresh-profile deferred-HTTP fixture, not physical-device acceptance.
- The unchanged connected v10 gate passed all 28 checks in 171.23 seconds
  against the exact candidate Controller, current supplied Machine and native
  pair below. Eighteen replies were held, four discarded through normal
  transport timeouts; exact command counts and fixture cleanup passed. Actual
  login, installation, logout refusal, retained native outcomes, uninstall,
  connection replacement and Controller restart remain accepted. These logout
  cases are a real chain regression gate, not HTTP role-mutation acceptance.
- All six embedded Agent releases have exact signed Catalog coverage. Candidate,
  active, next-transaction recovery and cold roles each passed two isolated
  reads of all 94 actual Catalog releases and the actual Service-owned host
  configuration preflight, before and after activation. Managed telemetry
  writer and standing background policy remain unconfigured. No durable format
  changed; historical journal matrices are not counted as newly run.

The supplied Machine is
`/nix/store/gh05dprk2zk8wqp6kjsiyn1ss30sdwb5-cowboy-machine-release`, source
`9ec6d7207f7a520085455c1e1f2a8036fcc278d8`. The exact native adapter is
`/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0/bin/cowboy-zed-adapter`
(SHA-256 `19ab0d291ccd51d9049d17eb88659effd3ee60634ec2365e20e77d8aff8b14ec`),
and server is
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`
(SHA-256 `f0d700155fb4b6fc1b25ec96d33d53ecd31e2278cab93f783def78003826930c`).
The receipt also hashes the supplied Machine wrapper chain, core adapter and
test-only LSP. Its temporary signed installation is not a production upgrade.

## Bounded production observation

Between `11:36:30.490Z` and `11:37:28.266Z`, all 16 original ACP workers retain
their PID, kernel start ticks and executable. The resident Machine remains
PID `3706189`, kernel start `349734872`, generation
`worker-f07cb63df88e764816f3`. All eight Plugin installation/version/digest/
incarnation/authentication identities, Machine/Web/host profiles and all three
Victoria processes remain unchanged. No retained worker was force-replaced.

Local and public `/healthz`, `/version`, `/` and `/sw.js` return 200. SPA and
service-worker bytes match the active Web files and retain `no-store`; version
remains `dfa174b1c916af496d77ef488fdad599`. Machine health is online/connected.
The Web release is
`/nix/store/8skwxqyyx97m60gk3a4syk9bjnci1lwk-cowboy-web-release`, source
`69e2c7034007f18efd8272828c2b8e7405f2d566`, independently activated at
`10:59:05.931835929Z` before this task's deployment window. That Web activation
and the earlier Machine maintenance are not actions of this Controller release.

## Evidence and remaining exits

Private evidence root: `/tmp/cowboy-permission-lifetimes.CjKreKyk`.
The bounded receipts, logs and observations have these SHA-256 identities:

| Evidence | SHA-256 |
| --- | --- |
| `negative.log` | `0cd7afd614fdd386c4d4de6d459a0983e394d8643f3588f05c69a5c680785fb7` |
| `check-final.log` | `3dbe99d07f028c299d3ed9e7b66e253162f6f35022809ab020131bde136d6804` |
| `build.log` | `c282190eed98a234c318e6ee9abb032bf0262855072100c56527543b6aa9f340` |
| `browser.log` | `6855109d431ba1eb615bb0bb3140a517c2b616b6d651d2d26d546a6c1075d0d9` |
| `connected-receipt.json` | `ab18d9d09c54adb06ffa088627de5bd62a6d50a0da9feba1d54e541b2ad4e0bc` |
| `floor-activation/floor.json` | `fd1d315f58aa8b6c2b578d086d60adf7f422563ce431abf9d08460e8afcf63da` |
| `floor-after/floor.json` | `169cf652c86cf0b8a8c64e6fc8384f441d21080a4653f9d3920603251935b671` |
| `observed-before.json` | `eb0ea25eb6d7a67d81a9613852f292565e183c43f5b9c0cab7fac3e5d714079b` |
| `observed-after.json` | `33acddb5144041f30e9bce55521f6a90013dfe8f29bde9662224c6ea9334b3ff` |

This closes core-observed product-role ABA for these finite consumers, not
unobserved database disable/re-enable, username/principal replacement, external
policy edits, persistent security-domain epochs, every legacy/streaming handler
or separate admin-cookie/host-delegation policy. General DAG resolution,
Machine-owned identity/state leases, retained-history/background-effect budgets,
Agent/native generation replacement, supported-device acceptance and independent
post-effect restoration remain in the
[completion ledger](../plugin-refactor-completion.md#code-work-still-required).
