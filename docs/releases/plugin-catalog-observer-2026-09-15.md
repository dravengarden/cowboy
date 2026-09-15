# Controller-owned Catalog observation release — 2026-09-15

Published and activated on Hawk from clean Cowboy revision
`c12a2d635404230f41f36c0494af9915a63d5758`, integrating the latest Catalog
watcher, six Agent dependency releases and the accepted decision to defer
unattended installation. The observer repair is commit `601d635b`.

- Controller release:
  `/nix/store/lvr03vnxgpqjm91dvf4mpif7niqi0r6k-cowboy-controller-release`.
- Executable:
  `/nix/store/rpxykznk3rab8m7q9j9cj7xbz2m558xr-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `9e09e4efed8d1e4268b6ad06672f517ad5860880fb302f6573ca3128ccd9eef3`.
- Transaction `1789469633187690049-c12a2d635404`: `succeeded`, `committed`,
  `published=true`.
- Effective predecessor:
  `/nix/store/46yq4vs3iv8vdlpxwagx8yy2j0aab9kw-cowboy-controller-release`
  (`d7bcc3cc`), not the older Controller observed at the start of development.
  That intervening release belonged to another task.

## Accepted scope

The [Catalog observer](../unattended-release-adoption.md) is owned by the
Controller. It observes existing public trust directories, caps burst settling
at three seconds, and uses bounded read-only metadata fallback for missing or
replaced roots and lost notifications. Unchanged accepted hints skip runtime
reconstruction. Stop suppresses new attempts; an already-started refresh drains
before normal storage teardown. A failed Provider projection after Plugin
Catalog commit is reported as that partial state, not a two-Catalog rollback.

This changes no signature trust, installation authority, journal format or
host-policy selection. It adds no Plugin package and installs nothing on a
Machine. Physical migration rollback, hard-kill recovery and independently
authorized post-effect restoration remain separate work. Unattended installation
layers 2 and 3 remain accepted and deferred, not enabled by directory
observation.

## Verification

Both the initial and integrated `nix develop -c just check-compact` gates
passed: format, strict Rust lint, dependency checks, independent feature graphs,
structural composition conformance, tests and shipped builds. The all-feature
Rust suite passed **1,263** cases with 29 explicitly ignored; the owned isolated
PostgreSQL gate separately passed all **17** PostgreSQL cases. The Web unit
suite passed **1,481** cases. These are not physical-device or real-browser
acceptance. Existing OTel spread, yanked transitive `spin` and bundle-size
warnings remain; none was suppressed or changed for this release.

There are **19 new tests** beyond the original four watcher tests. Continuous
events starving refresh and public-key directory changes not waking the reader
were reproduced as failures before the fix. Coverage adds source budgets,
missing/replaced roots, metadata-only observation, ignored artifacts/private
subtrees, signal loss, owner disposal, queued/retrying work, draining active
refreshes, unchanged-hint suppression and recovery without new notifications.
Actual signed Catalog fixtures accept new releases, reject invalid signatures
without ending original leases, and recover a Provider failure after Catalog
commit. The targeted Catalog suite passed 54 cases; its two ignored PostgreSQL
cases are included in the separate database gate.

The actual immutable candidate, then-current predecessor and cold Controller all
passed `serve --check-plugin-catalog` against the public Catalog, with a closed
environment and an absent disposable Service data path. They returned identical
**69 ready release identities**, including all six embedded Agent versions. No
Service state was created. This is actual signature-reader acceptance, not just
artifact presence or a synthetic key check. It does not verify runtime artifact
execution, host activation, migrations or real login. Cold remained
`/nix/store/cc09k6l788mhchy321ckgg0yryb1hg12-cowboy-controller-release`
(`869c269f`). The separate publication-coverage gate also accepted all six Agent
releases; this task did not publish those packages or exercise a production
installation. The historical 807-role installation/telemetry matrix was not
rerun or recounted here.

## Production observations

The pre-dispatch/after window was **18:53:38–18:54:30 +08:00** (52 seconds, not
an outage measurement). Controller PID changed from `2559799` to `2645039` and
its executable matched the accepted artifact. All **16 worker** PID/start pairs,
resident Machine PID `1232222` and all three Victoria process identities were
retained. Machine remained online on `worker-48ad34f5c4615668b75f` with its
workspace revision/hash unchanged.

Web/Machine profiles and receipts, host closure/unit hashes and cold roots were
unchanged. Both system and user failed-unit sets were empty before and after.
Local and public HTTPS checks accepted `/healthz`, `/version`, exact index,
admin, service-worker and both entry-asset bytes, including cache headers. SPA
version stayed `9dcf7c01602e4bf519e697e619761b03`; there was no Web activation.
The new observer logged successful reconciliation of 69 external releases at
18:54:04 +08:00. No production login, Plugin installation or managed Victoria
cutover was used as acceptance. Retained PIDs do not prove native-generation
upgrade, native resume or supported-device behavior.

Private evidence is retained at `/tmp/cowboy-catalog-lifetime-aVHpD92d`: failed
regressions, targeted and complete gates, immutable build, exact readers,
publication coverage, before/pre-dispatch/after snapshots and activation/HTTP
audit. SHA-256 identities:

- Integrated complete gate:
  `e4188106ce7f2ff7e6c6a4517c58d2e946dfddcda8db5c6f909da035aa048495`.
- Immutable build:
  `3802982931e4aac8b79e843f120575e148d68e7878464bf1da99b61611194284`.
- Actual-reader audit:
  `b7c077481c932bf9bb38fbae093e416fe41ca4ce936b120b9a272c53abf87ab8`.
- Activation/HTTP audit:
  `29fc92fdb91c71443cb2759646f597fbd802eaca31f74be7ba1b5ce330013525`.

General graph/site/state leases, independent post-effect/native recovery and
account/device acceptance remain in the
[completion ledger](../plugin-refactor-completion.md). Draining an owned task is
not cross-site compensation or generic DAG authority.
