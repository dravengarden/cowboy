# Core process cleanup and connected Code installation — 2026-09-16

**Controller published and activated; Machine remains an unactivated
candidate.** This release removes ambient signal-helper lookup and extends
connected Code acceptance to actual authenticated installation. It does not
switch ordinary Review, synchronize native text, publish a new Zed Plugin or
restore native resources after a crash.

## Source and artifacts

Product implementation is `8b7458267c725177735f1bfeac5e5de5de2a6398`. Three
following test-only repairs improve fixture diagnosis, allow holding the exact
installation reply, use the existing production installation budget and accept
an already-removed private probe directory during fixture teardown. Final
accepted and deployed source is `2fb32d523ec22f74983fdb6b41c4030dab975d3b`,
descending from freshly fetched `4c35c32d84a891eaec2d8817a10acaae0016b2ce`. The
enclosing documentation update changes neither application nor Plugin bytes.

- Controller:
  `/nix/store/n2r5956hr0ac0xiqvg3d04qmgivipgak-cowboy-controller-release`.
- Executable:
  `/nix/store/mz69j5m8kxwa249snhckmvasy7v20nch-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `4db76aed939d2f185b610c9da1f46edca6f5088cb34dff853a2728ec7388f962`.
- Machine candidate:
  `/nix/store/f64bmxcrnyxw5fh5kz9r4ws5yria4i3l-cowboy-machine-release`,
  generation `worker-df292108ca42dc0667e7`.
- Unchanged private Zed adapter:
  `/nix/store/c9rmd5rx6bfw131l3zcm7cri62k477f4-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.6.0/bin/cowboy-zed-adapter`,
  SHA-256 `58ddcff029845d8b6e00890aafe48c89ac2373d3bdde4c3728e5764aebd66bc0`.
- Unchanged server:
  `/nix/store/xjhjhq461q2qfwir9vmwbnaq7qxp3v11-cowboy-zed-server-1.13.0/bin/cowboy-zed-server`,
  SHA-256 `5829fe9d9f0b7a5a27129dc217cc9954c3b4334da5da2426bffe55423e723ae5`.

The first passing connected run used the same product implementation in the
`8b745826` Controller/Machine releases (`8f93kla3…` and `gv38yf3l…`). The second
used both final releases above. Both receipts bind exact executable and wrapper
hashes. The identical Controller executable hash does not substitute for these
separate provenance-bound runs.

## Accepted changes and gates

[Process cleanup](../plugin-process-cleanup.md) now uses the already-present
`rustix` process-group calls. It rejects broad/overflowed selectors, holds the
direct-worker mapping through the syscall and treats only `ESRCH` as absence.
Missing or hostile PATH no longer blocks group termination or invokes an
untrusted `kill`. The tests preserve unrelated children and distinguish
permission denial from absence. Cgroup fallback remains best-effort; there is no
new claim of PID-reuse immunity, escaped-child containment or recovery. The
cropped Machine source explicitly includes the new test module.

The [connected gate](../plugin-code-connected-conformance.md) starts with an
empty Code installation slot. A disposable signed artifact server supplies the
exact native bytes. Actual HTTP admission performs the installation; anonymous
access is refused. Losing the HTTP observer after the held original Machine
receipt does not cancel settlement. Repeating the same operation ID returns
completed evidence with execution refused, not another install command.

Both actual-process runs pass all **eight** groups, including the prior
content/read/release/uninstall/connection-replacement cases. The final run has
59 correlated replies, four held replies and exactly one install observation,
one install step, three opens, two releases and one uninstall step. Cleanup and
acceptance are true. Forced private fixture teardown is not product recovery;
the remaining owner is not represented as released after Controller restart.

The complete pinned-shell `RUST_TEST_THREADS=1 just check-compact` gate passed
on clean final source: **1,357 main Rust tests, 313 standalone Machine tests, 26
core-adapter tests, 56 private Zed tests, 1,599 Web tests and 17 isolated
PostgreSQL tests**, plus format, strict Clippy, types, features, dependency and
Plugin/component checks, structural composition conformance and shipped builds.
Main/Machine retain 31/two explicit ignored tests; connected acceptance above
runs separately. Full `nix flake check` passed, including component builds,
private Zed integration and the narrow source boundary. Existing dependency,
lint and chunk-size warnings were not suppressed. This is not physical-device or
production account acceptance.

Four actual Controller roles read identical **82-release** signed Catalogs and
return byte-identical actual-Service configuration preflight reports: candidate,
active/next recovery `fc86l6vr…` (`f16361c9`), retained previous `i4jccmj6…`
(`5ea1b6ee`) and cold `cc09k6l7…` (`869c269f`). Source Agent publication
coverage also passed. No Service state was created by these checks, and no
configuration environment was printed or recorded. This change adds no durable
format, migration or writer policy; the historical installation/telemetry matrix
was not rerun or recounted as fresh acceptance.

Earlier fixture failures remain recorded: the new reply kind was absent from the
relay hold allow-list, and a 12-second read wait was unsuitable for the existing
90-second installation budget. No production timeout or automatic retry was
changed. The first configuration-preflight invocation also stopped at Deno's
`/proc` permission check before running a candidate; the accepted invocation
used the reviewed script with the needed local permissions. Nix's existing
hardlink-limit warnings were avoided with per-command
`auto-optimise-store=false`, not host configuration changes or store deletion.

## Production result

The machine-owned installed activator committed transaction
`1789550880146294310-2fb32d523ec2`, `succeeded`, `published=true`, with the
accepted active reader above as its automatic predecessor. Only Controller
activation was dispatched. Web, Machine, Plugins, host policy, authentication
and Victoria configuration were not changed.

The observation window is **17:27:47–17:29:47 +08:00**, not an outage duration.
Controller PID changed from `1332600` to `1557251` and matched the candidate.
All **13 worker** PID/start-time pairs, resident Machine and Victoria processes
were unchanged. Machine remained online on `worker-3a889de3bf203a2378b8` with
the same workspace identity/revision. Machine/Web profiles and receipts, host
closure/unit hashes, cold roots and both failed-unit sets were unchanged. The
pre-existing failed system units were not cleared by this task.

Local and public `/healthz` and `/version` passed; index, admin, service worker
and both entry assets matched the unchanged immutable Web bytes. HTML/SW are
no-store and hashed assets immutable. SPA version remains
`ef6c61e69546aa10e3e0f93c6309aad8`; this task requires no new PWA bundle reload.
Retained process identities do not prove an upgraded native generation or actual
native resume.

## Evidence and remaining work

Private scratch evidence is `/tmp/cowboy-plugin-finish-UUbkIQUB`, not a durable
public artifact. Selected SHA-256 receipts:

- First connected acceptance:
  `bc8ac01cb5ab2d83b5ea131d51e0cd9ecda82e3a5cdc896ffd883c15cd0bd98b`.
- Final connected acceptance:
  `6fd3b418baea4ed979bee328d56cb34bdfc36f5dc11c429d8db01a3048a92ea9`.
- Complete gate:
  `e8f66332371ad065b8668ca775e340a70557dd5699cfc1114787fefee71c51d1`.
- Full flake gate:
  `24c664ad696e670c0fab1e02bfead2222e0dd1c8422f9736b49917bf8b65956e`.
- Actual-role Catalog audit:
  `f7cdc20da5575007975503166ab418f3e686e4a178a6b3440f2de3f1038147de`.
- Component/HTTP acceptance:
  `f225d932f298337610ac7d3d02a2bf8cec420ef396f78e06f74ac6ccd9c8bff5`.

The [native synchronization investigation](../plugin-native-buffer-sync.md)
identifies an upstream protocol gap: read-only preflight plus an unconditioned
reload can overwrite intervening native edits. Maintaining a private conditional
server primitive is a new delivery decision, not an already-accepted fix.
Review, owned navigation, independently authorized Machine/Code activation,
post-effect/native recovery, broader graph/state leases and account/device
acceptance remain open in the
[completion ledger](../plugin-refactor-completion.md). This release does **not**
complete the entire Plugin refactor.
