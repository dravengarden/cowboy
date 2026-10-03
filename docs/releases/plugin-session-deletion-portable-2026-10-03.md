# Portable Session deletion refusal gate — October 3

Updated portable host selection now refuses committed terminal Session state
until portable reader/recovery admission exists. Signed Machine-host payloads
cannot be fetched, probed or published through this path while a committed
deletion entry is present. Newly generated launchers refuse before selecting
active or bootstrap executables. Production writing remains disabled.

## Behavior and scope

The read-only gate accepts absent, empty and staging-only namespaces. It refuses
any committed entry, including a directory or dangling symlink, and refuses
invalid namespace entries or inspection errors. It neither parses nor rewrites
the record, creates a floor, authorizes a reader, or reconstructs historical
deletions. A same-user dataset removal remains outside this finite check.

Component reconciliation checks before download, after signature verification,
before the health probe and before publishing active/rollback/command links.
A record created during the probe refuses publication. Cached verified payloads
and probe effects may remain; rejection is not rollback. Other component kinds
retain their existing signed activation behavior.

Install/register and refresh check before local identity, bootstrap or launcher
mutation. New launchers first execute the installer-owned bootstrap diagnostic
`--check-portable-session-deletion --state-dir ...`. This returns before opening
stores, enrollment or a Controller connection. A failed or unsupported
diagnostic prevents both active and bootstrap host execution.

Welcome reconciliation now restarts the host only after an accepted component
batch. A rejected host candidate previously still requested exit 75; retaining
the resident reader avoids a rejection/restart loop. Explicit Reconcile uses
the same accepted-result condition.

The current Nix component owner retains its separate root-owned reader floor.
Its launcher is not rewritten by this release. Older portable installers and
launchers, older host activators, a persistent portable reader floor and signed
portable recovery selection remain unadmitted. Linux source execution does not
establish macOS/device acceptance or native resume. No constructor or release
metadata enables the writer.

## Source verification

Pinned-shell Machine-host library tests passed 465 with five ignored. The
all-feature unit suite passed 1,772 with 42 ignored; enabled integration and
doc-test targets passed. Clippy with warnings denied, formatting and diff checks
passed. The opt-in immutable release-pair target remains separately ignored.

Six new unit tests cover the read-only namespace gate, refusal before host
fetch, record creation during a signed probe without link publication, retained
unrelated payload admission, installer refresh before mutation and rejected
Welcome restart selection. Existing signed-payload coverage now also executes
with a terminal journal present.

The actual installer-generated launcher integration uses the compiled native
diagnostic and temporary HOME/state. Staging-only selection succeeds for both
active and bootstrap cases; committed file/directory/dangling cases refuse
before either selected marker executes. No provider-usage database appears.
A bootstrap diagnostic exiting 23 prevents active execution. The fixture uses
no real Controller, Provider runtime or production dataset.

## Immutable release and activation

Source `14cb070e0ab4e5ea5dfbdbd1287cf1b80c7b19dc` is published to remote
main. Concurrent main integration added Matrix conformance documentation/tool
changes without changing the tested Rust source. The clean committed narrow
Machine release and source-boundary output built successfully:

- `/nix/store/ypqs95ri4mv39bqx9lhknn2zcz4yrn3k-cowboy-machine-release`.
- `/nix/store/54g96bk7hymx36az3jqagapzlk4f4ihr-cowboy-source-boundary`.

The default-feature Cowboy package required by that immutable bundle passed
1,311 unit tests with 24 ignored and its enabled three-test integration target.
The packaged Machine diagnostic passed six isolated cases: absent state,
staging-only, committed file, committed directory, dangling committed link and
invalid namespace file. Accepted diagnostics created no Machine state/stores;
staging bytes remained unchanged. These probes used temporary directories.

Machine-only transaction `1791022091513469183-14cb070e0ab4` committed
successfully, published, without recovery at `2026-10-03T10:08:19.038530178Z`.
Its previous release is
`/nix/store/n6b8rxna00v77pyqkyk9658xcwnqnh61-cowboy-machine-release`.
No Controller, Web, host configuration or portable-device activation was made
by this task. This supplies the updated guard for future portable bundles; it
does not rewrite already installed portable launchers.

Integration of previously published worker changes advanced the desired
generation to `worker-6ede7a91cc8b8b3402d4`; this guard itself changes no
worker-generation input. The public deployment-health endpoint reported the
Machine connected and online with that exact active generation. This is a
generation report, not proof that every existing worker has been replaced.

Before/after observations at `2026-10-03T10:07:07.687Z` and
`2026-10-03T10:08:24.420Z` retained every original PID across 13 ACP worker
and four execution keeper units. Machine PID changed from `70549` to `929157`;
Controller PID remained `486493`. Web profile and root-owned reader-floor bytes
were unchanged. These are bounded samples, not native-resume acceptance.

HTTPS health, version, SPA, service worker and Machine deployment-health all
returned 200. HTML/SW retained `no-store`; the separate SPA version remained
`4aae6c684d7dd1914dd7520ccfa82cb4`. The resident reader logged zero deleted
IDs with `writer_enabled=false` at `2026-10-03T10:08:11.615527Z`. Its
production deletion namespace still contained only `.lock`; no records were
seeded, migrated or written, and production deletion remains volatile.

The [receipt and bounded observations](../experiments/plugin-session-deletion-portable-2026-10-03.json)
retain exact artifacts, native digest, process samples and packaged diagnostic
results. Older recovery authorities and writer admission remain separate.
