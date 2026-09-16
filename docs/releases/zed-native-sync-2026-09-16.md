# Private native synchronization and Hawk rollout — 2026-09-16

**Zed 1.7.0 is signed, published and installed on Hawk.** The separately
authorized Machine maintenance is also committed. The private conditional
primitive is not a public synchronization grant: ordinary Review still uses
its legacy reader, and independent restoration remains unfinished.

## Source and immutable release

Implementation and final artifacts use published Cowboy commit
`f56e629b83c10c17ea8533dcc7d87ad30fb9168c`, rebased onto freshly fetched
`ddcab9a5`. This enclosing evidence update does not change those runtime bytes.
Only Linux x86_64 is declared. Communication, installation and authorization
remain core mechanisms; ordinary Zed configuration and binaries are untouched.

| Identity | Accepted value |
| --- | --- |
| Plugin / adapter | `zed` / `cowboy-zed-adapter`, `1.2.4` → `1.7.0` |
| Private server | `zed-remote-server 1.13.0` → `cowboy-zed-server 1.0.0` |
| Upstream source | `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`; source-owned additive overlays |
| Publisher | `cowboy-first-party`, configured Ed25519 identity, independently selected public verifier |
| Composite artifact | `sha256:8cead0f4df841e0cc6e2b2fe3d579daf04a354dfd7f648d2704ab95a7a435dfe` |
| Package | `sha256:9c2690e323bd1f57bd3cfd0f97f252ff1d90a186dd6c7f387cb423e56c2929ad` |
| Adapter ELF | `sha256:af8412382564f526ec9522f7b362574e5d3ebf0dc8cda99c31f74ef634dc09b3` |
| Server ELF | `sha256:da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |
| Contract fingerprint | `sha256:9d2a7c2a0b0e69cf2413b5370741b3461ff386220c710d4875e4bc0722430733` |

The old release digest was
`sha256:3b850f191d6d9b97bbcb9ec64b59c6f227c91a8b005be5e7881f6965bea0ce75`,
with fingerprint
`sha256:8a4eb07205fd692909bae57b65c8c1e36c8e8f8614868e4a49bee73a82a0f4ab`.
The patched distribution has its own dependency name/version; upstream's
`version` response remains `1.13.0`. No SDK version rule was weakened to accept
a prerelease suffix. The bounded filesystem helper uses upstream-locked
`rustix 1.1.2`, not a runtime helper executable. Both final ELFs have no
interpreter or shared-library dependency. Workspace Git/language tools are a
separate trusted-workspace concern, not an ambient adapter/server resolver.

Immutable build results:

- Adapter: `/nix/store/gcss0fz3a0ccg3z4nq458hxxcjrrs26x-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.7.0`.
- Server: `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0`.
- Activated Machine: `/nix/store/yrdndji8fd7v7hc9a2r35a59l12dqll4-cowboy-machine-release`.

## Acceptance and an actual predecessor failure

The [native contract](../plugin-native-buffer-sync.md) has six native
GPUI/filesystem tests, deterministic barriers at both asynchronous boundaries,
bounded safe source reads, exact version/owner checks, original-ID outcomes and
no replay. The actual private-server gate passes native event/content
propagation, edit/undo/close refusal, invalid source, lost reply, duplicate and
process-instance-loss cases. Test barriers are absent from the shipped server.

The final static pair also passes signed temporary install, uninstall drain and
retained-generation reactivation. Two eight-group connected Code gates pass:
first with the earlier accepted cleanup Machine candidate `f64bmxcr…` (`2fb32d52`),
then with the exact final `yrdndji8…` Machine. Both use the actual Controller
release `n2r5956h…` (`2fb32d52`), disposable enrollment/password login and an
empty installation slot, not production credentials. Cleanup is accepted; the
fixture's forced exceptional cleanup is not product restoration.

A separate run using the then-active Machine `lzzinbc0…` (`0a6c572e`) failed at
connected installation after 96.5 seconds. Its retained receipt says
`accepted=false`, `failure=timeout`, with one installation observation, one
step and no completed checks. That older core still resolves `kill` through
PATH before waiting for the probe leader; the isolated fixture supplies no such
helper. The candidate uses the already-tested process-group syscall repair.
This predecessor failure was not relabeled as a passing release gate or worked
around by relaxing the fixture. No production 1.7.0 installation was attempted
on it.

Before maintenance, the final Machine, actual predecessor and actual cold
bootstrap `j7lix2f4…` passed all **72 installation-reader checks**. The real
Machine-owner policy preflight returned the exact schema and `unconfigured`;
no telemetry writer policy was enabled. This does not re-accept every unrelated
production policy or claim a new durable journal format.

The complete integrated `RUST_TEST_THREADS=1 just check-compact` gate passed:
**1,357 main Rust, 313 standalone Machine, 26 core-adapter, 62 private-adapter,
1,605 Web and 17 isolated PostgreSQL tests**, plus format, strict Clippy, types,
feature graphs, live dependency checks, Plugin/composition conformance and
shipped builds. Main/Machine/private-adapter retain 31/two/two explicit ignored
tests; applicable actual-process gates ran separately. Full `nix flake check`
passed, including immutable components and private Zed integration. Existing
lint/chunk-size and dependency warnings were not suppressed. A narrow documented
`cargo-machete` exception covers the actual `prost-build` build-script use.

## Publication and production installation

The canonical release skill required staging and independent verification before
live Catalog writes. Three actual immutable Controller roles read the complete
**83-release** Catalog twice before and twice after publication: active/next
transaction recovery `n2r5956h…`, retained historical predecessor `fc86l6vr…`,
and cold `cc09k6l7…`. Their actual-Service configuration-only preflights also
passed; no environment or credentials were recorded. A historical predecessor
is not misidentified as the next transaction's automatic recovery target.

The package and both runtime HTTPS URLs were fetched and matched their exact
signed hashes. Only Zed was published. The primary Catalog advertises `1.7.0`,
`ready`, Code payload 2, SDK 1.8, Linux x86_64 and the exact fingerprint above.
No Agent login or authentication contract applies to this Code Plugin.

Machine transaction `1789561022817231305-f56e629b83c1` is
`succeeded / committed`, `published=true`, and reports
`worker-4208d4d141de95cf9feb`. It uses the installed machine-owned maintenance
activator, not a unit override, full NixOS switch or installation-pointer edit.
It brings the prior core Code/UTF-8/ownership and process-cleanup changes into
the resident Machine as well as the new source provenance.

Zed installation used the enabled host Operator through the existing durable
Service/Machine installer, with exactly one submitted operation:

- Operation: `hawk-zed-1-7-0-native-sync-8cead0f4df84`.
- Response: HTTP 204; Service `completed`, Machine `applied`.
- Installation: `installation-f5f8f7645aebedc9f3fe567a2e650189674c77745588459b7021d65bfb606e88`.
- Inventory: `1.7.0`, exact new digest/fingerprint, active, no reconciliation
  fence; `1.2.4` retained as the rollback generation.

A local CLI invocation omitted the `operator` subcommand and failed before any
installation request or operation intent existed. Correcting that invocation
did not create an alternate installation ID or retry an ambiguous effect.

The observation window is **20:16:59–20:21:33 +08:00**, not an outage duration.
All **15 worker PID/start-time pairs** survived the immediate Machine restart.
One later completed the ordinary generation cutover: the exact new executable,
original resume-argument hash and native resumed-session hash all match. The
other **14** retained their PID/start identities. This is bounded lifecycle
evidence, not proof of a subsequent user turn or complete history, and not all
workers have drained to the new generation. No worker was force-stopped by this
task. The four private Zed processes were intentionally retired by Machine
maintenance; they are not claimed retained or restored.

Controller/Web profiles, processes and receipts, Victoria processes, host/cold
closure, unit hashes, failed-unit sets, workspace identity and all other Plugin
installations remained unchanged. Local/public health, version, five exact
SPA/admin/SW/entry files and their cache headers passed. Web version remains
`8581ce4be562a393fe0c0efc01e769f7`; this task did not deploy a new PWA bundle.

## Evidence and exclusions

Private evidence root: `/tmp/cowboy-native-sync-wJXWdz9O`. Selected SHA-256:

| Receipt | SHA-256 |
| --- | --- |
| Final connected gate | `cef8c7b66ff7dcde09ab2c078f70838d0f2c39c3a47dee8f54198f841df80a42` |
| Failed predecessor gate | `0951962b25e90256dc1f805f45948c2961e2c1726f97c2717114499478e4a2ab` |
| 72 Machine reader checks | `0a7bd1bfb1708cd78cf31de4d90798a11989aff47d76973b3ecc60729d4a0632` |
| Complete source gate | `5bf2cec1eb178d35d3d9762d5696b6d8c7496996341c714bc6cea52e7dbc6e53` |
| Full flake gate | `8782c814e4be1e1884f092c5db7d4e4b6ce70513eb48fabb307f8f5f07eccc4b` |
| Production activation audit | `a42b9e33e09b071cc107f323df2885936adfeb2e3a68f676ac8668e13980d965` |
| Native resume metadata | `0666e959a6ce32f69e66b13992170974cf9a8dcbd368cd53f16919183d367b36` |

The public immutable publication receipt lives under
`/var/lib/cowboy/plugin-catalog/receipts/zed-1.7.0-8cead0f4df841e0cc6e2b2fe3d579daf04a354dfd7f648d2704ab95a7a435dfe.json`.

Still required: adapter multi-owner exclusion through unknown outcomes, core
synchronization purpose/authority, actual Review and owned-navigation cutover,
independently authorized post-effect restoration, general live graph/state
leases, managed Victoria policy/ingestion cutover and supported account/device
acceptance. A single native peer is not exclusive adapter ownership. Native
process loss is not restoration. The [completion ledger](../plugin-refactor-completion.md)
therefore remains open; this release does not complete the whole refactor.
