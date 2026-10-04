# Portable Machine cached startup integrity — October 4

The portable launcher previously checked the deletion namespace, then executed
`components/commands/cowboy-machine` without authenticating the cached host.
Staging verification did not protect a subsequent restart against changed bytes,
manifest or selection pointers.

Machine host reconciliation now retains the authenticated original artifact in
the generation's `artifact` file, using an exclusive random temporary file and
atomic replacement without following a cached destination link. After a signed
probe, it also authenticates
this proof and manifest and requires the decoded manifest to equal the candidate before
publishing pointers. Archive expectations come from the authenticated original
archive, never from a mutable cache inventory. Other component kinds are
unchanged. Retained artifacts count toward the existing cache pruning budget.

The trusted installer-owned bootstrap's existing offline diagnostic now checks
any cached Machine host before the launcher changes PATH or creates its runtime
directory. Both selection pointers must exist as symlinks, select the singleton
host at its exact version/digest path under the regular component directory
tree, and agree on the authenticated executable. It rechecks the publisher
signature and signed reader declaration, original artifact SHA-256, exact raw
bytes or archive tree, and executable permissions. Proof files are opened with
no-follow/nonblocking flags; manifests are bounded to 64 KiB. It creates no
Machine stores and runs neither a cached probe nor cached host. The configured
publisher key is passed explicitly by generated launchers. No network download
is needed for startup verification.

Absent active and command pointers retain trusted bootstrap selection without
requiring a publisher key. Incomplete, dangling or substituted selection,
missing artifact proofs, invalid signatures, changed bytes/companions, links and
non-executable hosts refuse startup without fallback or repair. Legacy caches
need fresh signed reconciliation to obtain an authenticated retained package.
The diagnostic reports `host_cache_guard: 1`; installer bootstrap probes now
require that capability as well as the existing committed-state refusal before
install/refresh changes bootstrap, identity or launcher configuration. A previous
deletion-only bootstrap is deliberately incompatible with a new installation.

This does not authenticate the caller-selected bootstrap or its publisher-key
configuration. The user's trusted administrative authority is retained. Checks
observe mutable files at a point in time and do not fence concurrent privileged
writers. Archive resource budgets, persistent portable floor, signed bootstrap
and recovery admission, direct caller-owned host launch, and production deletion
writing remain separate. Committed portable deletion state still refuses before
cache selection, and the production deletion writer stays disabled.

## Validation

Source fixtures cover raw/archive cache authentication and corrupt manifest,
retained proof, original artifact digest, executable, companion, symlink,
missing/redirected/outside/dangling pointer and execute-bit refusal. Correct and
wrong publisher keys are exercised. They prove cached probes are not executed,
selection links are unchanged on refusal, and absent state is not created.
Signed probes modifying the manifest or retained artifact cannot publish.
Installer fixtures reject a deletion-only bootstrap capability report.

An opt-in exact-release fixture runs generated launchers with the immutable
Machine diagnostic over fourteen isolated raw/archive cases. Its ordinary-start
bootstrap marker detects unintended fallback; cached-host markers detect code
execution. Healthy cases execute only the selected host. Changed manifest,
package, payload/companion, missing legacy proof, redirected command and committed
journal cases refuse before creating `run`. A second fixture checks the actual
new bootstrap and rejects the preceding immutable deletion-only release. These
are finite Linux startup checks, not native Session resume, power-loss acceptance,
or a production writer-release gate.

## Accepted source and release

Source `b4d651951910c04fb00d11b3a429876295050d62` is published on remote
`main`. Formatting, all-feature Clippy with warnings denied, 1,789 all-feature
library tests (44 ignored), integration tests, 481 standalone Machine tests
(7 ignored), and Machine/Code feature checks passed. The full immutable default
Cowboy package and source-boundary check also built with their normal warnings
and test gates; the worker-registry immutable check passed.

The initial all-target gate exposed the old launcher fixture's acceptance of
an unsigned executable command file. Its expectation now asserts refusal,
while bootstrap and signed-cache positive controls remain. Initial immutable
builds caught an incorrect Rustix API spelling and an ungated test helper in
the default feature slice; both were corrected. No compiler warning, signature,
namespace or production writer gate was relaxed. The opt-in integration test
was rerun with its required all-feature binaries after a Machine-only command
could not compile the independent ACP worker binary; no worker feature or
retained-bundle policy changed.

Exact native acceptance used
`/nix/store/9rqny80agvhq8s5ni7ddacj8bps1m18b-cowboy-machine-release`, the
preceding deletion-only release
`/nix/store/z4pmjs2j2jy5093vljzzif72vmxbxicz-cowboy-machine-release`, and the
pre-guard legacy installer release
`/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release`.
All fourteen generated-launcher cases and the new/old bootstrap capability
control passed. The exact new installer also passed both legacy layout refresh
controls, both captured mutable-bundle controls and both independently invoked
old-installer negative controls. Its release entrypoint SHA-256 is
`6f8ce2981b9304a7047972985da21ae9608560b0cbfb0444628b73660325fc84`.
Independently selected old installers retain their administrative authority;
these negative controls do not claim to fence them.

The activated release is the exact accepted output of
`.#cowboy-machine-host-release`. Its native executable is
`/nix/store/8laiznd3bykmi497i9i1pnrd3y3qzw8j-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`,
SHA-256 `def7e43bbfeb07c0234131da0a8f993322b13532f89334241f5ba6597f035cd6`.
All six retained worker/proxy/Code/Zed/JS companion paths and digests equal the
previous active bundle. Their independent source stays
`406471a28de430debf6f8363b44abc3e621589d7`, generation
`worker-6ede7a91cc8b8b3402d4`.

## Production receipt

Installed-owner transaction `1791088757650881684-b4d651951910` succeeded,
published and committed at `2026-10-04T04:39:26.704395709Z`, with
`maintenance: true` and no recovery. Its previous release is the exact
preceding `z4pmjs2j2jy5093vljzzif72vmxbxicz` output. The installed activator
and host configuration were unchanged; the root systemd activation unit
finished successfully.

Samples at `2026-10-04T04:38:44.715Z` and `2026-10-04T04:39:53.585Z`
preserve all thirteen worker and five keeper unit IDs, states and PIDs exactly.
Only the resident Machine changed, from PID `1442220` to `2088278`.
`/proc/2088278/exe` matches the accepted native path and SHA-256. Controller
PID `959309` and receipt `1791079277476212104-406471a28de4` were unchanged.
This is bounded Linux process continuity, not general native Session resume.

All five public HTTPS health/version/SPA/SW/Machine checks returned 200.
The Machine reports connected and online with the retained generation.
HTML/SW kept `no-store`, and the Web profile and SPA version were unchanged.
Root reader-floor SHA-256
`26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`
and sudoers SHA-256
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`
remain unchanged. Both component in-progress journals are absent; no failed
system unit was observed or reset. At `2026-10-04T04:39:17.734667Z` the
Machine logged zero deleted IDs and `writer_enabled=false`; its deletion
namespace remains only `.lock`.

The [machine-readable evidence](../experiments/plugin-host-startup-integrity-2026-10-04.json)
records exact build/native/retained identities, source checks, owner, successful
transaction and complete before/after process and HTTPS samples.
Existing remote portable installations must refresh their bootstrap and launcher
to use this guard; this Hawk activation does not rewrite those installations.
Portable persistent floor, signed bootstrap/recovery and committed-state
admission remain closed, and the production deletion writer remains disabled.
No Controller, Web, host configuration, installed Plugin or iOS activation was
performed by this task.
