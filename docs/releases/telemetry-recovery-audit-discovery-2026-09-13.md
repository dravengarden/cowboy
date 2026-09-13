# Durable recovery audit discovery: Hawk acceptance

Accepted 2026-09-13. Controller, Machine and Web use clean, published Cowboy
revision `74a71b102de976659d457a6898a81608f7291f78`, integrating remote main
`35b8604b25d61be4423aec3ad65d5043f09e52a5`. The concurrent website logo/preferences
changes were preserved. See the [contract](../telemetry-recovery-audit-discovery.md).

This closes the process-local recovery query-handle gap. It does **not** open
production binding, Machine recovery, Service resolution or managed-export
admission, nor complete P2/the whole Plugin refactor. There is no new journal
format, migration, signed Plugin/SDK, Provider or native ABI change.

## Gates

`nix develop -c just check-compact` passed: 1048 Rust tests (19 deliberately
ignored fixtures), 1415 Web tests, 15 independently isolated PostgreSQL tests,
strict Clippy/type/feature checks, dependency/Provider/site gates, Web build and
release builds. The original test commit was rebased over the website-only
remote changes; `just site-check` passed again on the integrated source.
Nix Controller packaging independently passed 820 library tests (15 ignored)
and three command tests; the separate Machine and Web outputs built successfully.

New tests exercise complete query/receipt linkage, schema/purpose separation,
protocol/reply-kind/connection correlation, original deadline as historical
data, sticky evidence poisoning, actual Machine reopen and later heads, and
real cookie logout/role/account revocation before and after a read. A fresh
HTTP owner with empty plan registries discovers a reopened Machine audit after
separate Service resolution and a later Service operation without changing
either journal. Rust/Web share a closed public fixture.

The release evidence directory is private:
`/tmp/cowboy-recovery-audit.hIGfro/`. Earlier targeted/full-gate failed logs
are retained but are **not** acceptance; `check-compact-accepted.log`,
`check-integrated-site.log`, immutable build logs and the two accepted reader
receipts are the accepted evidence.

## Independent activations

All three machine-owned root transactions finished `succeeded` / `committed`,
with `published: true` and activation unit result `success` / exit 0:

| Lane | Transaction | Completion (UTC) |
| --- | --- | --- |
| Controller | `1789281019212973274-74a71b102de9` | `2026-09-13T06:30:43.861755779Z` |
| Machine | `1789281073952274768-74a71b102de9` | `2026-09-13T06:31:22.076281752Z` |
| Web | `1789281171079921180-74a71b102de9` | `2026-09-13T06:32:51.120511814Z` |

Active releases:

- Controller: `/nix/store/lsv34w1xcpz5f4z473w5kbf8s30x71ki-cowboy-controller-release`.
  ELF `/nix/store/34abn5aamprn7mnp7lkrx2kq0sx55mps-cowboy-0.1.0/bin/cowboy`, SHA-256
  `efa59abb316f1119b954e3346f385e302f49c593c9fc0f4df53dab9614a5ce9e`.
- Machine: `/nix/store/0x3zhdvqsa31w5rpvsjd9w7yl77w81ng-cowboy-machine-release`.
  ELF `/nix/store/5ijb9hsf60d2djfxx8czk5mj7s23y9y5-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`, SHA-256
  `547b217fc843ddca53715669af183629791f2c46fc2b4952a6ffc2438e402a39`.
- Web: `/nix/store/vxxxqk4sf1zqp8qahf1slkclqngj1q8d-cowboy-web-release`;
  served root `/nix/store/jpqdpfqi13bmgglmq4n6sqgab4s61cd2-cowboy-web-0.1.0`.

Historical predecessors, not the *next* transaction's default recovery targets:
Controller `/nix/store/w024allw47zf31clxvncyz941igvsykv-cowboy-controller-release`,
Machine `/nix/store/gwd83024a8dbi65srphcda55mssbkjh3-cowboy-machine-release`, Web
`/nix/store/f0gb4694anrdcq8m6hx785qcz03ys23q-cowboy-web-release`.

Controller restarted only `cowboy.service`; Machine used its explicit separate
maintenance lane and negotiated **protocol 18**, confirmed in its authenticated
connection log. Web then moved only its asset profile. Final Controller PID
`3814516`, start counter `2963953324900`; Machine PID `3817934`, start counter
`2964008082119`. The actual `/proc` executables matched the immutable ELF hashes.

Across the immediate pre-deploy capture and all three post-lane captures,
**all 15 worker unit PID/start pairs were identical**, SHA-256
`205fd96d84246247d5139f7e0606cca37db5769f0a529f6c7e13b3c25ba93c23`.
Worker generation stayed `worker-92b35f0665ec33ba60f6`; Hawk remained online,
workspace revision `de1f6c6504146de3633271fc223d3aaa3c449212`, workspace-ID hash
`109fda30f8dad466162ab87d6c6b41fd8de2d20d149a548a2644602f594e23bd`.
This is deployment-window continuity, not a claim about every worker's lifetime.

## Actual reader recovery floor

The pre-cutover matrix tested candidate readers, the actual active predecessors
as rollback readers, and the real absent-profile cold outputs. All **96/96**
checks passed, including 12 protocol-18 Machine checks. After all activations,
the actual active and next-transaction rollback readers are the active releases
above; both were tested again, with **96/96** passing and 24 protocol-18 Machine
checks. The cold Machine's 12 protocol-17 reads remain explicitly distinguished.

The unchanged active NixOS closure is
`/nix/store/4rp5n3nx5hjpccjc2fjlr62vrfhx8bwv-nixos-system-hawk-26.05.20260731.5b4f72e`,
source `de1f6c6504146de3633271fc223d3aaa3c449212`. Its actual activation script pins
Controller cold reader `/nix/store/lynj6vqp7ph7mcn82rwggkwcjzkyn37m-cowboy-controller-release`
and Machine cold reader `/nix/store/dxwq5k1iab6wsyj7gg24gqndwz9han08-cowboy-machine-bootstrap-release`.
No host configuration or cold-floor refresh was performed.

Final private, create-only, mode-0600 schema-two receipt:
`dist/telemetry-reader-conformance/20260913-recovery-audit-active.json`;
SHA-256 `be56499ec81ba8b9ac1835ea471de51d7eebd46f4eeb6acc3348f7c14205908c`.
Its source is the deployed revision, `accepted: true`, with zero failed checks.

## Public health and remaining scope

Public `/healthz` returned `ok`. Root and `sw.js` returned 200/no-store;
`/version` and root ETag were `fa687acd0eeae1e1fd488ec7ed6fd471`, SW ETag
`d27bb4e7342762844e70b4e856699976`, and both shell references use `cowboy-v1674`.
An unauthenticated durable audit GET returned 401. Installed PWAs need their
normal update/hard reload; a WebSocket reconnect alone does not update JS.

Production startup still reported `admission_enabled=false managed_namespace=false`;
the Machine binding file remained absent (and not a symlink) throughout. No
production binding/recovery was created as a smoke test, and no private telemetry
policy, credential or endpoint was copied into evidence. Physical-device and
production write/fault-injection acceptance are not claimed.

Remaining P2: explicit per-target writer/background-export policy admission and
cross-end production write/failure/restart acceptance. Unknown/schema-one
recovery stays quarantined; already emitted OTel remains `NoRestore`.
