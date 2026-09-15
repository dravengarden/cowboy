# Service-bound Plugin Sites — Controller release, 2026-09-15

The [finite Service/Machine Site boundary](../plugin-service-sites.md) is
published and active on Hawk. A core-loaded Service identity now owns the
communication registry. Resolution, preflight and both outgoing paths reject
foreign Service/Machine claims before dispatch. This closes another finite P0
boundary, not the whole Plugin refactor.

## Source and activation

- Runtime and immutable test harness:
  `fa1fd219bb46ed6675ea538f8d202c65cd6e61e7`, clean and published on main.
- Controller release:
  `/nix/store/qvr3sm7a4h1bbx54z1vqmjwsdf5sp03h-cowboy-controller-release`.
- Actual ELF:
  `/nix/store/h2afwnrn18i2wda867cai15b5l3h3xpa-cowboy-0.1.0/bin/cowboy`.
- ELF SHA-256:
  `623b55bfff8c8351a6bbcc6dcd90b3c7e4ecea66d058ae433a79cf4f4f255bf2`.
- Transaction: `1789447712651079995-fa1fd219bb46`, `outcome=succeeded`,
  `phase=committed`, `published=true`.

The task fast-forwarded fresh main to `92997a83`, preserving the independently
released product sign-in layout. The machine-owned component activator restarted
only `cowboy.service`. No Machine maintenance, worker rebind, Plugin release or
production installation, login, host-policy change or managed telemetry cutover
was performed.

## Acceptance

The pinned complete `just check-compact` passed: **1,199 Rust library tests, 285
standalone Machine tests, 1,481 Web tests, 17 isolated PostgreSQL tests**, 86
Rust/TypeScript structural differential vectors, format/Clippy, dependency
audits, native/source/feature/Plugin closure checks and shipped builds. Existing
Web lint/chunk and advisory-policy warnings were not suppressed. Seven new unit
tests cover Site isolation and identity lifetime. Three failed before the
repair; the command matrix adds 144 checks across twelve scoped commands, four
Service/Machine combinations and three outgoing entrypoint forms. This is an
internal invariant repair, not evidence of an exploitable public HTTP route.

All **807 immutable role checks** passed:

| Gate                                     |             Checks |
| ---------------------------------------- | -----------------: |
| Service installation readers             |                168 |
| Machine installation readers             |                 72 |
| Connected installation                   | 45 / 90 cold reads |
| Telemetry readers                        |                 96 |
| Background startup                       |                 78 |
| Telemetry writer admission               |                294 |
| Connected telemetry                      |                 45 |
| Victoria database ingestion/query/reopen |                  9 |

The existing five installation flows retain actual same-bytes reinstall,
installation incarnation, lost Applied receipt at the real 90-second deadline,
disconnection, Controller crash, two cold opens per reader pair and no replay.
Both prior Catalog-lifetime preflight probes still pass. Connected telemetry
retains independent confirmation/recovery, select/revoke/restore, all three OTLP
lanes, HTTP failure/partial-success refusal and no resend after lost ACK.

| Lane       | Candidate                                      | Next transaction recovery at dispatch          | Cold bootstrap                                 |
| ---------- | ---------------------------------------------- | ---------------------------------------------- | ---------------------------------------------- |
| Controller | `qvr3sm7a4h1bbx54z1vqmjwsdf5sp03h`, `fa1fd219` | `nsd4mn4g4cd6g99ykvpdg00xk7gsi9sq`, `a27f1742` | `cc09k6l788mhchy321ckgg0yryb1hg12`, `869c269f` |
| Machine    | `5wh7wikgqbl8r4ya927xh04dizqmjziq`, `6a1eff6b` | Same actual active release                     | `j7lix2f4wbp2dvbxs7hprmp5kzcr413n`, `869c269f` |

The table uses store hashes; receipts retain complete paths, manifests and ELF
identities. Matrix roles were independently bound to the actual profiles and
active host bootstrap closure before dispatch. The completed transaction's
historical predecessor was not mistaken for the next recovery target. After
activation, the new Controller becomes the ordinary next transaction's recovery.
Older binaries retain format compatibility without acquiring this new guard.

Victoria used the actual host services' immutable executable versions: Logs
1.52.0, Metrics 1.148.0 and Traces 0.9.3. Tests used isolated loopback,
disposable databases, signed fixture Plugins and temporary identities.
Production database storage, endpoint policies, accounts and credentials were
not used.

## Live verification and limits

The bounded observation window was **12:47:59–12:49:27 +08:00**, 88 seconds, not
an outage-duration measurement. Controller PID changed from `1718735` to
`1937491`; its executable matches the accepted release. All **15 running
worker** PID/start pairs, resident Machine and three Victoria processes were
unchanged. Machine remained online on `worker-48ad34f5c4615668b75f`, workspace
revision `80e6788785d0834154c40ae498b11128017a95ee` and the same workspace
identity hash. This is not native-generation swap or physical-device acceptance.

Web/Machine profiles and receipts, host/unit hashes, cold roots and both failed
unit sets were unchanged. The existing `xdg-desktop-portal-gtk.service` failure
was preserved; it was not reset or fixed. No new failed service appeared in the
window. Web stayed on its independently activated `92997a83` release,
`cowboy-v1691`, SPA version `9dcf7c01602e4bf519e697e619761b03`. Local and public
HTTPS health/version, index, admin, service worker and both entry assets passed
exact-byte and cache-header checks. This Controller release does not require a
new Web bundle.

No protocol, Plugin/component version, SQL migration, journal encoding,
identity-file format or policy changed. The Site guard is neither authority nor
a generic state lease; unscoped legacy wire commands retain their own domain
checks. Workspace/Session/security-domain resolution, verified graph contract
linking, independent post-effect/native recovery and real account/device/managed
cutover acceptance remain in the
[completion ledger](../plugin-refactor-completion.md).

Private evidence: `/tmp/cowboy-plugin-service-sites-M3UDaQWm`. The initial
unwrapped capture stopped because `jq` was absent; its empty partial directory
was retained, then the successful snapshots ran in the pinned shell. It is not
counted as an acceptance result. Complete create-only conformance receipts, logs
and `gates.sha256` remain under that private evidence root. Selected SHA-256
values:

| Evidence                         | Digest                                                             |
| -------------------------------- | ------------------------------------------------------------------ |
| Complete quality gate            | `6ef9b1af26f8afae5eaa42e07812da4e34db87851395cdbf599c630976c66d1a` |
| Eight-gate role audit            | `8cd9fdeb0f034a7f1c01b8c31f4080947bbdebde156f20ac177076cd7cc44a27` |
| Connected installation           | `7ed3bd15bf4aa952b4c614c3c6358d1c76362b5adccd791056a1678bb5b1b1c3` |
| Connected telemetry              | `093df2a7fbf020d4d8d146dd66755144c5bf795421430fe44f54827f3d52bf81` |
| Real Victoria pairs              | `25f3fc4474c70519f22f8e247f9061aaca218e8a556f4b60e41e7d18e90358cc` |
| Activation and public HTTP audit | `9ba9e324c412318ac0fa9ed8d1b2cb2f470629721cf0535dd5e8718f1ae8465e` |

This later documentation-only record does not introduce another activation or
another signed Plugin package.
