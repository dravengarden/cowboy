# Portable Machine cached startup integrity — October 4

The portable launcher previously checked the deletion namespace, then executed
`components/commands/cowboy-machine` without authenticating the cached host.
Staging verification did not protect a subsequent restart against changed bytes,
manifest or selection pointers.

Machine host reconciliation now retains the authenticated original artifact in
the generation's `artifact` file. After a signed probe, it also authenticates
this proof and manifest and requires the manifest to equal the candidate before
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

Build, exact-release acceptance and production receipt are recorded below after
completion.
