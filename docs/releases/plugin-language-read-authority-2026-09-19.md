# Legacy language read authority — 2026-09-19

Published and activated the Controller-only extension of
[buffered Code read authority](../plugin-code-read-authority.md#legacy-language-queries).
The four legacy diagnostics/hover/navigation/outline GET handlers now consume
the original product authority and Session/connection observation. Closed query
types replace handler-local untyped command construction. Revoked credentials,
lost Session visibility or a replaced route discard the complete reply without
retry, reload, acquisition or release by the Controller.

## Exact release

- Clean source and connected harness: `0f370f05b117ed8d56dc36aaf2fc2e9816a6a618`.
- Controller: `/nix/store/i2qh6vmffzrvd6hc40b5jcr2ch88bdkg-cowboy-controller-release`.
- Executable: `/nix/store/nnyc5vqrng0dzl7f35d79c60yz92vlqr-cowboy-0.1.0/bin/cowboy`;
  SHA-256 `8a75ad37f3e0a67b153cf687c5de5dfbd260298391d756f45ece4c7871b90381`.
- Previous Controller: `/nix/store/91k8n7l4whms3hi9idw0zi53fqrlkshi-cowboy-controller-release`,
  source `e66befc783984bc08c31ad771038e3fda63d7d41`.
- Activation `1789810713817762549-0f370f05b117` committed at
  `2026-09-19T09:38:52.327134731Z`: published, succeeded, non-maintenance,
  no recovery. Source was pushed to remote main before activation.

No Plugin, SDK, Machine protocol/native binary, SQL baseline, host configuration
or Web release changed. Subsequent documentation commits do not change the
accepted runtime artifact above.

## Gates

- Complete pinned-shell `just check-compact` passed: 1,541 main Rust tests
  (34 explicitly ignored), 375 standalone Machine tests (4 ignored), 26 core
  adapter tests, 126 private adapter tests (2 ignored), 1,833 Web tests and
  all 18 separately isolated PostgreSQL tests. Formatting, lint, types,
  feature/build gates and all 86 shared structural-link vectors passed.
- Focused reader suite: 43 passing tests. New tests cover closed request shapes,
  wrong response kinds, pre-dispatch/parked-reply connection replacement and
  Session cwd ABA. Nix artifact build: 1,157 default-feature tests
  (18 ignored) and three bridge tests passed.
- Connected v9: all 25 checks passed in 171.03 seconds using the actual supplied
  Controller, protocol-21 Machine and exact Zed pair below. Each legacy query
  has exactly three dispatches: independent-login read, held original-login
  read refused after API logout, and independent-login read afterward. A later
  request using the revoked login dispatches nothing. The complete chain still
  checks four actual lost replies through normal timeouts, one uninstall,
  connection replacement and Controller restart.
- Negative test: the previous immutable Controller returned HTTP 200/JSON after
  original-login logout at `legacy_language_original_login_revocation`.
  Acceptance failed as expected in 8.67 seconds, with exactly two language
  queries and verified fixture cleanup. No production login or database edit.
- All six embedded Agent releases have exact signed Catalog coverage.
- Immediately before activation and after it, candidate/active/next-transaction
  recovery/cold roles each passed two isolated reads of all 94 actual Catalog
  releases plus the actual Service-owned configuration preflight. The cold
  Controller remains `/nix/store/hn2zd44ngda15pz6ki1qdjdw6c7ifmfh-cowboy-controller-release`
  at `94382b94e8275c6ad21cfdb9da1907bf1dab8d7a`. Managed telemetry writer and
  standing background policy remain unconfigured. No journal/writer format
  changed; historical journal matrices are not counted as newly run.

Connected inputs retain Machine release
`/nix/store/rg1266170l053dkzb292b2hrlxq8xhyh-cowboy-machine-release`.
The exact native adapter is
`/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0/bin/cowboy-zed-adapter`
(SHA-256 `19ab0d291ccd51d9049d17eb88659effd3ee60634ec2365e20e77d8aff8b14ec`);
server is
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`
(SHA-256 `f0d700155fb4b6fc1b25ec96d33d53ecd31e2278cab93f783def78003826930c`).
These disposable native processes are not a production installation or recovery.

## Bounded production observation

Between `09:38:21.511Z` and `09:39:28.933Z`, all 16 original ACP workers
retain their PID, kernel start ticks and executable. Resident Machine PID
`2129810`, generation `worker-3a3c8b33f96774b1981e`, its component profile,
all eight Plugin installation/version/digest/incarnation/authentication
identities, Web and host profiles remain unchanged. This window starts after
the earlier, separately recorded Claude authentication-generation roll.

Victoria Logs/Metrics/Traces retain PIDs `573148/3740238/3740317` and kernel
start ticks `48436608/256114621/256114636`. The audit uses these stable kernel
identities, not wall-clock formatted process start times.

Local and public `/healthz`, `/version`, `/` and `/sw.js` return 200.
The SPA and service worker match the active Web files and retain `no-store`;
version remains `78e5e670596be11d547e59c646957c6c`. Machine deployment health
reports online/connected. The unchanged Web profile is
`/nix/store/49ws0s01kh84il32xqqihr4dvghscax0-cowboy-web-release`.

## Evidence and exclusions

Private evidence root: `/tmp/cowboy-language-read-authority.sh1qGcdj`.
The immutable connected receipts, actual-role reader reports and bounded
observations have these SHA-256 identities:

| Evidence | SHA-256 |
| --- | --- |
| `connected.json` | `d66ea745f68d76b615f9c8a99eeca3f80226c757c79f2dc9acae0c1e1499a7ba` |
| `connected-negative.json` | `9ef7681b2447dc4d237b765207accecc606a14d8be8f6fb70a73cecd81d67b88` |
| `floor-activation/floor.json` | `1680992c6a2a3475382720925888989dca9650298c9351bb5fe5c995bf5500e6` |
| `floor-after/floor.json` | `fe8288933c12c85de4351dcf21401d531c6460775003d1444cbfa53e90832b0c` |
| `observed-before.json` | `91cb74b93824ec093d031fbb664a9b7a07b41f50ad082bd2419b8094df754ca6` |
| `observed-after.json` | `6d321f5bc5f59ac7aea3377fb5138e621d3ca0e7d75f86a95b859b8c26aaa159` |

Development diagnostics remain alongside successful evidence: the initial
focused compile's trait-import error was fixed; the initial preflight helper
needed Deno's explicit procfs permission, and a temporary audit regex was
corrected before taking either deployment snapshot. Neither helper failure
was a product or continuity acceptance, and neither triggered activation.

This is a finite response authorization boundary, not continuous principal
epochs, state reader/writer leases, graph execution authority, native generation
adoption or independent post-effect restoration. Native legacy navigation may
acquire destination state internally: rejecting its HTTP result does not prove
those effects reverted. No physical-device/native resume acceptance, credential
roll continuity or full Plugin-refactor completion is claimed.
