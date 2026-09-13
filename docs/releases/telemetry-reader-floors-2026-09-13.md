# Populated telemetry reader floors — 2026-09-13

Hawk's active, next-transaction rollback and cold recovery readers now have
populated Service/Machine conformance evidence. **Managed telemetry writes are
still closed. This is not P2 completion or fleet-wide acceptance.**

## Repository gate

Cowboy implements the owned
[immutable-reader conformance](../telemetry-reader-conformance.md) in test-only
Rust modules, invoked through `just telemetry-reader-conformance`. There is no
new production CLI, HTTP mutation, protocol version, Provider/Plugin package,
native ABI or worker-generation change. The canonical release skill routes this
boundary through actual readers and independently verified host roles.

Implementation through `347558600d41a693992f23843d8e0a01d8d9b432` passed the
full pinned `just check-compact`: **1,012 Rust library tests passed**, 19
deliberately ignored; **1,371 Web tests passed**; all **15 isolated PostgreSQL
tests passed**. Six ordinary harness tests cover fixture writers, closed
matrix/environment, actual child environment, timeout/reaping, evidence
preservation and private create-only receipts. Two additional ignored tests are
the explicit artifact gate and its private hanging-child fixture. Subsequent
changes only ignore local private receipts and record this delivery.

The gate runs eight cases on each Site, twice on the same temporary state, for
all three supplied roles: **96 checks**. It compares exact Machine binding and
recovery observations over the existing read-only commands, checks the Service
startup fence as well as health, and verifies that no binding row/file changes.
Corrupt fixtures require their specific reader rejection, not just any process
failure. The closed child environment includes only a hashed pinned-shell
OpenSSH helper and explicit temporary paths.

Early fixture failures were not counted as acceptance. They exposed missing
explicit signing-tool setup and the initial empty Plugin inventory frame. A
later single handshake failure remained a failed receipt; diagnostics were split
by phase and direct-mode probes now wait for that child's fresh broker
readiness, not a stale socket left by the previous read. The resulting code
passed repeated complete matrices, including both actual cold artifacts before
and after the host transaction. It does not retry a failed check into success.

## Accepted actual artifacts

All four artifacts below embed the clean Cowboy reader revision
`d6d48b2abc902086ec929c81d6d46c78c63f14ea`.

| Site / role                                              | Immutable release                                                              |
| -------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Controller active and next ordinary transaction rollback | `/nix/store/jcavcj8p33qvxnazvvhiqr2p9bld8kl5-cowboy-controller-release`        |
| Controller cold                                          | `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`        |
| Machine active and next ordinary transaction rollback    | `/nix/store/gwd83024a8dbi65srphcda55mssbkjh3-cowboy-machine-release`           |
| Machine cold                                             | `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release` |

The cold artifacts were resolved from the **actual built and then active Hawk
closure**. Their ELF hashes differ from the standalone active builds because
Hawk follows its own Nix inputs; matching Git revisions alone was not used as
compatibility evidence. All launchers and ELF hashes are in the bounded receipt.
The Machine bootstrap is only a cold reader, never an activation candidate.

The completed component receipt's historical `previousRelease` is not the next
ordinary transaction's rollback target. The activator captures the current
profile at the new transaction boundary. Existing profiles and component
receipts were unchanged here, and no incomplete component journal existed. No
redundant Machine maintenance was performed to rewrite a historical field.

The negative matrix used the retained Controller `e3e4dace`, retained Machine
`51f79972`, and actual prior cold Controller/Machine `6a420ff5`. It failed with
**48 rejected checks out of 96**, preserving nonzero exit status. The old cold
Controller can serve health while ignoring even populated/corrupt telemetry
authority. Old Machines lack the required protocol or fail to open new records.
These binaries are not accepted post-write recovery targets. Neither evidence
nor migrations were deleted or rewritten to make them work.

## Owned Hawk recovery transaction

Columbus commit `de1f6c6504146de3633271fc223d3aaa3c449212` is published to main.
It changes only the Hawk Cowboy recovery input, its generated lock and exact
reader check, and the recovery runbook. It integrates fresh main and the active
host revision. `just verify`, the exact recovery-reader Nix check, and the owned
full-system build passed. The merged system/user units were compared: only the
generated `mandb.service` path differed; Cowboy and network units were
identical.

The independent host activator recorded:

- Transaction `1789233515931768231-de1f6c650414`.
- `outcome=succeeded`, `published=true`, at `2026-09-13T01:18:42+08:00`.
- Active closure
  `/nix/store/4rp5n3nx5hjpccjc2fjlr62vrfhx8bwv-nixos-system-hawk-26.05.20260731.5b4f72e`.
- Previous closure
  `/nix/store/z1ffzg6ry8id9wbapkcgbzm2pgdnfrs9-nixos-system-hawk-26.05.20260731.5b4f72e`.
- Only changed unit `mandb.service`, no explicit restarts or newly failed units.
- Required Cowboy and retained model-gateway health checks passed.

The receipt is `/var/lib/hawk-deployments/current.json`. Independent
before/after checks prove Controller PID **2088827**, Machine PID **2091295**,
and all **15** worker PID/start records unchanged. All three Cowboy profiles,
their component receipts, Web root, version and cache/ETag headers compare
unchanged. These independent comparisons, not the host receipt's empty
changed-retained-unit list, establish Cowboy continuity.

Hawk remains connected/online at `worker-92b35f0665ec33ba60f6`. Its workspace
revision hot-reloaded to the new Columbus commit; the workspace-ID hash stayed
`109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`. Root and SW
return 200/no-store, with unchanged ETags `b85d8b88a592838909a87e3c5c9326f9` and
`fa8019e41386eaaf3c506379f9e63f8a`. The Machine binding namespace remains
absent. No production database, private endpoint/token policy or Provider/native
state was read or changed for this gate.

## Evidence and remaining admission

Private local receipts under `dist/telemetry-reader-conformance/`:

- `20260913-active-rollback-cold.json`: accepted 96/96, SHA-256
  `3683b16efde0ae99af798c8a7f112322cc572adb98aed5ee39b34d98e3d0aa4f`.
- `20260913-old-reader-negative-control.json`: intentionally rejected matrix,
  SHA-256 `6f779a9970b9ac600fcea01394ddfb302e5e1d2ed895d1bd4932175ad52df81c`.

Detailed gate and host comparisons remain under
`/tmp/cowboy-telemetry-readers.MwkzDF/`. Worker PID/start evidence has the same
SHA-256 before and after:
`a9660c473ec7f3599c1fbb71a91a021e1ae77757f51760bbb82db71d5e16560c`.

This accepts the populated reader floor for Hawk's Service and Hawk Machine
Sites, subject to rechecking changed artifacts/roles. Falcon and other targets
are not covered. Finite user-facing confirmations, explicit production writer
and managed background-export policy admission, and complete production
cross-end failure/restart acceptance remain. Existing explicit Victoria routing
and rotating local telemetry were not migrated. Unknown evidence stays fenced;
already emitted OTel remains `NoRestore`, not a reversible external effect.
