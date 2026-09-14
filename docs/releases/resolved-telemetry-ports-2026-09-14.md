# Resolved telemetry ports — Controller release, 2026-09-14

The [finite live telemetry resolver](../resolved-telemetry-ports.md) is published
on Cowboy main and active on Hawk. It connects exact verified contracts and
original per-installation observation leases to the existing binding/export
executors. This accepts that finite path, not general executable DAGs or the
whole Plugin refactor.

## Source and immutable activation

- Clean, published Cowboy source: `6fd7f2cac168a692706d5ea087d58a34c3f09ca3`.
- Controller release:
  `/nix/store/54qr9mz395ziaib7hkmg0d8r673ivh6w-cowboy-controller-release`.
- Actual executable:
  `/nix/store/2njf3n5ydj61m21viywjf7b6fkjb0g8n-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `a694ec56487a30c54df972e21d8e267604851c3013d0e9dc12a4564c82e808b3`.
- Transaction: `1789393157299108785-6fd7f2cac168`,
  `outcome=succeeded`, `phase=committed`, `published=true`.

The machine-owned `cowboy-controller-activate` restarted only `cowboy.service`.
No Machine maintenance, Plugin installation/publication, host-policy change,
production login, database migration or managed telemetry cutover was performed.
Existing legacy Victoria export and its configuration remain unchanged.

The activation's recovery target was the then-active Controller
`/nix/store/987xsvdchhvh4bhp8qf04680p15zsp7g-cowboy-controller-release`
at `4817bc7f6ca095aa8de279ec6c9287261be865e6`. It is format-compatible but lacks
this new observation-continuity repair. After successful activation, the new
Controller is the normal next transaction's recovery target; the recorded
`previousRelease` is historical, not an instruction to deploy an ancestor.

## Acceptance

The pinned complete `just check-compact` passed:

- 1,178 Rust library tests, 285 standalone Machine tests, 1,461 Web tests;
- 17 isolated PostgreSQL contract checks;
- format, Clippy, dependency/license/advisory policy, source/feature boundaries,
  component/package closure gates, native-shell and site checks, the unchanged
  86-vector Rust/TS composition differential gate, and shipped builds.

Eight new tests cover verified release/fingerprint requirements, unique active
installation and per-slot isolation, original exact input/purpose, legacy/OTLP
separation, all three signals, Catalog refresh and terminal failure, and atomic
enqueue after an observation discontinuity. Both binding and export regression
tests failed on the original implementation; after repair they reject all nine
tested away-and-back discontinuities without sending an effect. A development
test initially failed to compile because a pattern guard moved its receipt;
the final test borrows it. No gate or runtime requirement was relaxed.

Clean-source immutable process acceptance additionally passed **45/45 connected
flows** and **9/9 real Victoria database pairs**. Each Controller role crossed
each Machine role:

| Lane | Candidate/active | Transaction recovery | Cold bootstrap |
| --- | --- | --- | --- |
| Controller | `54qr9mz395ziaib7hkmg0d8r673ivh6w`, source `6fd7f2ca` | `987xsvdchhvh4bhp8qf04680p15zsp7g`, source `4817bc7f` | `l4ji30l5pysy8091kwxmz8igsvvpgv95`, source `2f7fa237` |
| Machine | `pzs2k265fb4klvxd4bx6r5fzsw8harjf`, source `4817bc7f` | Same actual current release | `7m0kq3x3hygg5f23l0yg4rcalfh2riw8`, source `2f7fa237` |

These are Nix store hashes; the private receipt contains complete paths,
manifests and executable-chain hashes. The recovery roles were bound to actual
pre-dispatch profiles, not the older historical `previousRelease` fields.
Both cold roots were checked against the active host's bootstrap activation
script. The host closure remained
`/nix/store/y9gs82a0pki3r30z1k7r1nv1hx08gwng-nixos-system-hawk-26.05.20260731.5b4f72e`.

Connected flows cover select/revoke/restore, genuine disposable login,
connection replacement, lost ACK at the real 45/15-second deadlines, independent
recovery/resolution, restart without replay and bounded managed delivery faults.
The separate database gate used the same immutable executable versions as the
running services: VictoriaLogs 1.52.0, VictoriaMetrics 1.148.0 and VictoriaTraces
0.9.3. All nine pairs validated logs, metric values and correlated traces,
repeated semantic queries after database reopen, and no replay after Cowboy
restart. Both gates used isolated processes/storage/network and fixture keys;
neither used production credentials, data or destinations.

## Production verification and limits

The measured window was **21:39:05–21:39:44 +08:00**, 39 seconds. Controller PID
changed from `3820978` to `147162`, whose actual ELF matched the release.
All **14 worker** PID/start pairs, resident Machine and three Victoria process
identities were unchanged. Machine stayed connected with generation
`worker-240c2080a8bf9eb8968f` and workspace revision
`8e358baba640f361c39f26a49390741cfd3a972e`.

Web/Machine profiles and receipts, host/unit files, cold roots and failed-unit
sets were unchanged. Web retained the independently published theme-default
fix at `a00aac6f59cf83901f68ec44b878ab92acb885f9`, service worker `cowboy-v1687`,
SPA version `a5a575af00bf603ea4bdab3e66cdee38`. Local and public HTTPS index,
admin, service worker and both entry assets matched the active immutable Web
bytes; document/SW `no-store` and asset `immutable` cache headers passed.
Both `/healthz` and `/version` passed. No PWA reload was needed for this
Controller-only change or performed as device acceptance.

No protocol, signed Plugin/component version, journal encoding, SQL baseline or
policy was changed. A slot observation lease is not a durable state lease, and
resolution still grants no effect authority. Already emitted telemetry is
`NoRestore`. General graph/scope/dataset resolution, independent post-effect
Plugin/native recovery, actual Operator/managed policy cutover, supported-client
acceptance and the other [completion exits](../plugin-refactor-completion.md)
remain open. PID continuity in this window does not prove a native generation
swap or session restoration.

Private local evidence root: `/tmp/cowboy-telemetry-resolution.SLaraq`. SHA-256:

| Evidence | Digest |
| --- | --- |
| Complete source gate | `b1f088fe0d98ac62f3af2e32f5e13ff41354e9f6874dd64afa44da9cc97a4fb0` |
| Connected 45-flow receipt | `c2fe29131664a66be6d015e718cce63ad8566b2c67451f3942798f9b96f38c32` |
| Victoria nine-pair receipt | `8855919e226c81c89e5e00c4b0f55230eadb555d96c90b8d7b09dd171f4b0d1d` |
| Immutable gate audit | `4008181d6f8a5037da7a14a2f2229e352ace55477e77321fa160de0623c1c53b` |
| Bounded activation audit | `de2f9b6d53a464fc03900c140d2e671db07c15e1d6d6ab7212b5efbbcc6a998f` |

This publication record is a later documentation-only commit; it does not
represent another activation or a new signed Plugin release.
