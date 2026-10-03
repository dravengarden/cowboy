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

Exact build, activation and bounded continuity observations are recorded after
the clean committed Machine artifact completes its owning release transaction.
