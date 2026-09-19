# Owned buffer outcome authority — 2026-09-19

Published and activated the Controller-only extension of
[core buffer ownership](../plugin-controller-buffer-owners.md). Ordinary Open,
Query and Release now share one original credential continuation and an absolute
request deadline. Current original-user Operator authority is checked before
admission, before native dispatch and before returning an outcome, including
saved observations and remote failures. Synchronization/navigation reuse the
same authority check without changing their operation state machines.

The owned task records valid native effects before the observer checks permission
to disclose them. Logout or HTTP cancellation cannot erase an actual Open or
Release. A fresh original-user login can observe the same resource ID and
explicitly clean up its original owner; neither refusal nor duplicate observation
replays a mutation. A saved Open is historical evidence, not a fresh Session or
native-use grant. Unknown effects remain unknown, never implicitly compensated.

## Exact release

- Clean source and connected harness: `34ade0039fce3fae1a95ff902cd913d7743c1165`.
- Controller: `/nix/store/pqn8jrmmxv75z9f8ylss7g5zhj5r5pqs-cowboy-controller-release`.
- Executable: `/nix/store/cg4dm7apaxvy6fasx9rb4p1rfavxmacv-cowboy-0.1.0/bin/cowboy`;
  SHA-256 `fee5e99a435ab594abd758a1db75c239e593ff419d5a7feaf56157389ded6c1c`.
- Previous Controller: `/nix/store/i2qh6vmffzrvd6hc40b5jcr2ch88bdkg-cowboy-controller-release`,
  source `0f370f05b117ed8d56dc36aaf2fc2e9816a6a618`.
- Activation `1789815133119122906-34ade0039fce` committed at
  `2026-09-19T10:52:31.188136104Z`: published, succeeded, non-maintenance,
  no recovery. Source was pushed to remote main before activation.

This task changes no Plugin, SDK, Machine protocol/native binary, SQL baseline,
host configuration or Web release. Subsequent documentation commits do not
change the accepted runtime artifact.

The final evidence commit was rebased onto the independently published Web
change `69e2c7034007f18efd8272828c2b8e7405f2d566` after a non-fast-forward push
refusal. Its changes are preserved. The integrated Web passes typecheck, lint,
all 1,832 tests and build; Rust/core/Plugin sources are unchanged from the
accepted runtime. This documentation integration neither redeploys Controller
nor activates the independent Web change. Physical iPhone acceptance remains
separate from that source gate.

## Gates

- Complete pinned-shell `just check-compact` passed: 1,545 main Rust tests
  (34 explicitly ignored), 375 standalone Machine tests (4 ignored), 26 core
  adapter tests, 126 private adapter tests (2 ignored), 1,833 Web tests and
  all 18 separately isolated PostgreSQL tests. Formatting, lint, types,
  dependency, feature/build gates and all 86 structural-link vectors passed.
- Focused buffer suite: 70 passed. New tests exercise each operation across
  cookie/PAT revocation, account disablement and Operator-role loss, saved
  outcome refusal, malformed native replies and expiry before effect admission.
  Nix artifact build: 1,161 default-feature tests (18 ignored) and three bridge
  tests passed.
- Isolated Firefox `153.0.1`: all 24 buffer-owner cases passed, with fixture
  SHA-256 `4e7a11c5ac7d212fa57370ad912bca000d784143febfa4d683eece9bd7b2da27`.
  These use deferred HTTP and a fresh browser profile, not a production account
  or supported physical-device acceptance.
- Connected v10: all 28 checks passed in 171.31 seconds with the retained
  supplied Machine release `/nix/store/rg1266170l053dkzb292b2hrlxq8xhyh-cowboy-machine-release`.
  Each added Open/Query/Release case holds a real native reply, revokes only
  its disposable product login and requires `401/no-store/no-ETag`. An
  independent original-user login observes the retained outcome without
  replay; an explicitly queried/released owner still uses its original ID.
  The complete chain retains four lost replies through normal timeouts,
  uninstall, connection replacement and Controller restart.
- A second complete v10 run passed all 28 checks in 171.16 seconds with the
  actual current Machine artifact
  `/nix/store/gh05dprk2zk8wqp6kjsiyn1ss30sdwb5-cowboy-machine-release`, source
  `9ec6d7207f7a520085455c1e1f2a8036fcc278d8`. This additional supplied-artifact
  compatibility check completed after Controller activation; it restarted no
  production process. Both positive runs have 18 held replies, four discarded
  replies, exact command counts and verified fixture cleanup.
- Negative test: the previous immutable Controller returned HTTP 200/JSON after
  original-login logout at `owned_open_original_login_revocation`. Acceptance
  failed as expected in 8.74 seconds, with actual fixture cleanup. No production
  cookie, database edit, synthesized native ACK or timeout reduction was used.
- All six embedded Agent releases have exact signed Catalog coverage. Before
  and after activation, candidate/active/next-transaction recovery/cold roles
  each passed two isolated reads of all 94 actual Catalog releases and the
  actual Service-owned configuration preflight. Managed telemetry writer and
  standing background policy remain unconfigured. No journal/writer format
  changed; historical journal matrices are not counted as newly run.

Both positive runs and the negative supplied-artifact run use the exact adapter
`/nix/store/ng88l6jhmpnk07rxjwmz2s00l6llnwk0-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.20.0/bin/cowboy-zed-adapter`
(SHA-256 `19ab0d291ccd51d9049d17eb88659effd3ee60634ec2365e20e77d8aff8b14ec`)
and server
`/nix/store/i9yxq38r7pzhdkmp4bq8sypihaqlnxpq-cowboy-zed-server-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-server`
(SHA-256 `f0d700155fb4b6fc1b25ec96d33d53ecd31e2278cab93f783def78003826930c`).
Their temporary signed installations are not production upgrades or recovery.

## Bounded production observation

Between `10:51:44.439Z` and `10:53:19.754Z`, all 16 original ACP workers
retain their PID, kernel start ticks and executable. The resident Machine
remains PID `3706189`, kernel start `349734872`, generation
`worker-f07cb63df88e764816f3`. All eight Plugin installation/version/digest/
incarnation/authentication identities, Machine/Web/host profiles and all three
Victoria processes remain unchanged. No retained worker was force-replaced.

The Machine was already on
`/nix/store/gh05dprk2zk8wqp6kjsiyn1ss30sdwb5-cowboy-machine-release`, source
`9ec6d7207f7a520085455c1e1f2a8036fcc278d8`, before this Controller deployment
window. Its separate maintenance transaction `1789814391558015460-9ec6d7207f7a`
committed at `10:39:59.82877773Z`; it is not a Machine upgrade performed by this
Controller task or evidence of native-owner resume.

Local and public `/healthz`, `/version`, `/` and `/sw.js` return 200. The SPA
and service worker match the active Web files and retain `no-store`; version
remains `78e5e670596be11d547e59c646957c6c`. Machine deployment health is
online/connected. The unchanged Web release is
`/nix/store/49ws0s01kh84il32xqqihr4dvghscax0-cowboy-web-release`.

## Evidence and exclusions

Private evidence root: `/tmp/cowboy-buffer-outcome-authority.WrOccuM6`.
The immutable receipts and bounded observations have these SHA-256 identities:

| Evidence | SHA-256 |
| --- | --- |
| `connected.json` | `3df2a2bc64f44cd90c25e7ff1eee76116c0db2946b1e663bfd578c1da7af6980` |
| `connected-active-machine.json` | `7e5546930527a6b03b235b5c4aa26986777e81dac8bf935882d4fde21d1e04b0` |
| `connected-old.json` | `05b75bd19d351045381ae53e0747cfe58f96dbe06cf065852ba7a182268b3d72` |
| `floor-activation/floor.json` | `d75c929ac409c91ce06f6843bcbe1d040c7263f825f867b5306fd7fd3ee02c6c` |
| `floor-after/floor.json` | `849f86793af2941d40426c36d2a9903ee35cc2a0638dd3c0bfc90adba7a552f8` |
| `observed-before.json` | `9bebc8fbfb0e17d8a49b5d23fb4f88c8f873db9f59ba37a10ccfc62bdc99c6d1` |
| `observed-after.json` | `8719f8940cc574992c6737f42601cd3331310690170992c3a09c623bc5444aae` |

The initial focused compile's missing test import was corrected before all
passing gates; its diagnostic remains in the private evidence directory.
This is finite outcome authorization, not continuous security epochs, state
reader/writer leases, general graph execution, abandoned-browser/restart
restoration, native-generation adoption or independently authorized post-effect
recovery. None of the broader [completion exits](../plugin-refactor-completion.md)
is closed merely by preserving an effect receipt or denying its response.
