# Verified release observations — Controller release, 2026-09-15

The [core release observation lease](../plugin-release-leases.md) is published
and active on Hawk. Exact signed Catalog identity now remains continuous across
the finite install, uninstall and telemetry executors' awaits. An accepted
removal/re-addition cannot revive old confirmation, even when the bytes match.
This is a finite P0 improvement, not completion of the Plugin refactor.

## Source and activation

- Runtime source: `a27f1742e12282c21891913c9a2b71fae9873ed2`, clean and
  published.
- Controller release:
  `/nix/store/nsd4mn4g4cd6g99ykvpdg00xk7gsi9sq-cowboy-controller-release`.
- Actual ELF:
  `/nix/store/8d4qksblmk1537wlpvd68409da9q70z0-cowboy-0.1.0/bin/cowboy`.
- ELF SHA-256:
  `370eeb593763d4d65f20421027b55b15c8a8fdc2c5c43617dfa3c071d80acbb1`.
- Transaction: `1789439444977952325-a27f1742e122`, `outcome=succeeded`,
  `phase=committed`, `published=true`.
- Final test harness: `99e6dcc2fcadf29cf38e7d186ef1435de1414a35`. This
  descendant changes only test fixtures/helpers and documentation, not runtime
  behavior; the matrices deliberately execute the exact runtime artifact above.

The task integrated fresh main at `ed8bad98`, retaining the independent iOS
keyboard fix. The owned component activator restarted only `cowboy.service`. No
Machine maintenance, Plugin package publication/installation, native rebind,
host policy change, production login or telemetry cutover was performed.

## Acceptance

The pinned complete `just check-compact` passed, including **1,192 Rust library,
285 standalone Machine, 1,481 Web and 17 isolated PostgreSQL tests**, the
86-case Rust/TypeScript composition differential gate, format/Clippy, dependency
policy, source/feature boundaries, Plugin closure checks, native-shell/site
checks and shipped builds. Existing advisory-policy warnings and Web chunk-size
warnings were retained, not suppressed. Ten new tests cover Catalog lifetime,
both telemetry regressions and the closed fault relay; the existing fixture test
also verifies independent Admin credentials with no pre-issued session.

All **807 immutable role checks** passed:

| Gate                                                             |             Checks |
| ---------------------------------------------------------------- | -----------------: |
| Service installation readers                                     |                168 |
| Machine installation readers                                     |                 72 |
| Connected installation, five flows and two opens per reader pair | 45 / 90 cold reads |
| Telemetry readers                                                |                 96 |
| Background startup                                               |                 78 |
| Telemetry writer admission                                       |                294 |
| Connected telemetry, five flows                                  |                 45 |
| Victoria Logs/Metrics/Traces ingestion/query/reopen              |                  9 |

The installation flow additionally holds a real target-observation reply and a
real uninstall-preflight reply while authenticated Catalog refresh accepts
removal and restoration. Both old requests are refused without a durable intent
or Machine effect; fresh install and same-bytes reinstall succeed. Separate
probe counters require exactly two correlated queries/replies. Unknown outcomes,
lost receipts at the real 90-second deadline, Controller crash, independent
reader opens and no replay retain their previous checks.

The initial process test correctly encountered the Catalog's stronger Admin
boundary: a Product Operator cannot refresh it. The final harness preserves that
denial and performs a separate real fixture Admin login. The first failed
fixture receipt is retained, not counted as product regression evidence. With
the corrected harness, predecessor Controller `10898c82` fails the new ABA flow
while its other four flows pass; the candidate passes all five. No production
authentication rule or gate requirement was weakened.

| Lane       | Candidate                                      | Next transaction recovery at dispatch          | Cold bootstrap                                 |
| ---------- | ---------------------------------------------- | ---------------------------------------------- | ---------------------------------------------- |
| Controller | `nsd4mn4g4cd6g99ykvpdg00xk7gsi9sq`, `a27f1742` | `8zlgchrpyxd01ba037whwsk4zp3l0bwx`, `10898c82` | `cc09k6l788mhchy321ckgg0yryb1hg12`, `869c269f` |
| Machine    | `5wh7wikgqbl8r4ya927xh04dizqmjziq`, `6a1eff6b` | Same actual active release                     | `j7lix2f4wbp2dvbxs7hprmp5kzcr413n`, `869c269f` |

These are store hashes; receipts contain complete release paths, manifests and
ELF-chain hashes. Actual profiles, next-recovery choices and the active host's
cold bootstrap paths were checked before dispatch. Historical `previousRelease`
fields were not used as future recovery targets. The old readers are format
compatible but lack this repair; after successful activation the new Controller
is the normal next transaction's recovery target.

Victoria fixtures used the same immutable executables as the host services: Logs
1.52.0, Metrics 1.148.0 and Traces 0.9.3. They used private disposable storage,
signed fixture Plugins, genuine temporary identities and isolated loopback
networking, never production destinations or credentials.

## Live verification and limits

The observation window was **10:30:32–10:31:42 +08:00**, 70 seconds, not a
measured outage duration. Controller PID changed from `1400876` to `1718735`,
whose ELF matched the accepted artifact. All **15 running worker** PID/start
pairs, resident Machine and three Victoria process identities stayed unchanged.
Machine reconnected online with generation `worker-48ad34f5c4615668b75f` and
Columbus revision `80e6788785d0834154c40ae498b11128017a95ee`.

Web/Machine profiles and receipts, host/unit hashes, cold roots and system
failed units stayed unchanged. Web remains `cowboy-v1690`, SPA version
`93b263ba9fe56223c7046f63626d34c5`. Local and public HTTPS health/version,
index, admin, service worker and both entry assets passed exact-byte and
cache-header checks. No browser upgrade or physical-device acceptance is
inferred.

The **user failed-unit set did change**: `xdg-desktop-portal-gtk.service` failed
at 10:31:17 with `cannot open display`. Previous-day logs show the same
recurring headless failure. Its causal trigger was not established; the unit was
not reset and no host configuration was changed. The activation audit explicitly
records this exception instead of claiming all failed-unit sets were unchanged.

No protocol, Plugin/component version, SQL migration, journal encoding or policy
changed. Catalog checks do not claim immediate trust-file revocation or atomic
remote cancellation after dispatch. Already emitted telemetry is `NoRestore`.
General graph/scope/state leases, independent post-effect/native recovery and
real account/device/managed-policy acceptance remain in the
[completion ledger](../plugin-refactor-completion.md).

Private evidence root: `/tmp/cowboy-plugin-release-leases-Y3LlMFxS`. SHA-256:

| Evidence                                          | Digest                                                             |
| ------------------------------------------------- | ------------------------------------------------------------------ |
| Final complete source gate                        | `a991dc668be96e146229507d9ee3af81cded086d9ed64ceeaa69d13be3c90889` |
| Eight-gate role audit                             | `facab9272ccc1d55b89bd78903dfba5c7c63b73f7f2e83172aa476408c6da2b3` |
| Connected installation                            | `3cd4daa02fbf2d87f6dc60adfb7af782c21932246bd4c8968c5e20c7476d6dd3` |
| Corrected predecessor negative control            | `d3dc3c022bebdfe65f6d5c0f2f29d08e842bc139f4bb4aa1c860ec6e64e0ffdd` |
| Connected telemetry                               | `5d06c29053c3c0db422fc7c55f2438fd1659d7db24a3eccf93d5ac2088e5a052` |
| Real Victoria pairs                               | `ae99613f0c4f62b35ab4850d26c14fb4b822d49c16d61083250a739142ee44a3` |
| Activation, public HTTP and explicit host warning | `0983f95d394d2e78f8211fb8ea39ba81dfdd0dab69e0a02f91f7ab301bf464d7` |

This record is a later documentation-only commit, not another activation or a
new signed Plugin package.
