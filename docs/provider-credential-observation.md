# Provider credential observation

The Service owns credential generations. A Machine observes a changed runtime
projection, submits a candidate against the signed generation it read, and
converges to the Service's compare-and-swap winner. Runtime directories retain
native session state; observers do not choose a winner by file timestamp or
copy credentials between live sessions.

## September 2026 failure

On Hawk, a Claude runtime refreshed at 15:00 while other runtime generations
retained the credential that expired at 15:05. Subsequent failures at 15:21 and
15:42 cleared those generations' tokens. The Machine still held Service
generation 5. Its recursive authentication watcher covered 169,194 directories
under projected homes, including native history, caches and plugin trees.
Filtering each notification rediscovered contracts through full runtime archive
integrity verification. The queue was unbounded, and draining it had no bound.
The Machine had accumulated more CPU time than elapsed wall time.

A credential-file symlink alone is insufficient: native CLIs can save by
renaming a temporary file over it. Existing tests preserved this candidate for
Service reconciliation, but did not test the actual notification path amid
unrelated native state. An additional failure was that structurally present
credential files, including a CLI's signed-out document, could become candidates.

## Shared refresh serialization

A rotating credential is single-use: the first refresh invalidates the token
every other process still holds. Native CLIs coordinate this themselves, but
they scope that lock to the directory their credentials live in. Cowboy gives
each auth generation a private runtime home (CR-9) while linking all of them to
one credential source, so per-home locks do not intersect and two live
generations can refresh the same token at once. The loser then fails to
authenticate mid-turn and writes a cleared document, which this observation
layer correctly rejects — repairing the projection, not the race.

A Provider therefore declares where its CLI must find the shared store. The
`credential_directory` runtime binding names a declared credential file; the
Machine binds the directory of the projection that owns those bytes, for the
worker and for Plugin hosts that run the same authenticated CLI. Every live
generation then locks and re-reads one directory, and the native "another
process refreshed it" path resolves the race instead of failing a turn.

The binding is additive to Machine contract 4: an older Machine cannot parse a
package that uses it, so deploy Machines before publishing such a Plugin.
Session history stays in each generation's own home; only the credential
directory is shared. `native_claude_shared_credential_store_conformance`
accepts a new native CLI only while it still honours that directory.

## Boundaries

- Watches are nonrecursive and limited to declared credential paths, their
  ancestors and shallow Provider/generation discovery directories. Plans are
  cached for event filtering. Access events and unrelated native files never
  enqueue work. A capacity-one notification channel represents a rescan of
  current state, not a queue of historical credential values.
- Watch plans use authenticated package descriptors. Complete executable and
  archive verification remains mandatory for installation, launch and native
  probe execution. Credential observation does not need to reread those bytes.
- A fixed debounce cannot be postponed by a continuous stream. A bounded
  metadata reconciliation once per minute covers lost notifications and
  directory recreation; it never walks a Provider home. Newly discovered
  directories are watched before credentials are scanned.
- The Controller connection owns and cancels its observation task. Blocking
  filesystem and integrity work runs outside the async transport executor.
- Each complete changed projection is considered independently. A missing
  sibling cannot hide a valid rotation. A Plugin declaring a signed native
  exit-status authentication probe must accept the exact candidate in a private
  temporary home before promotion. The probe has no inherited Provider
  credentials, discards output, has a deadline and is killed on cancellation.
  Legacy releases and Plugins without this probe retain structural validation;
  this is not a claim of upstream network authentication for every Provider.
- Invalid candidates are excluded. While a valid candidate awaits CAS, failed
  siblings cannot request restoration of the expired baseline over it. Pending
  inventory is scoped to the generation actually submitted.
- The Machine publishes that inventory before its refresh candidates. The
  Controller must see the current Plugin version and auth generation before
  validating a candidate; a trailing pending inventory could otherwise overwrite
  a completed reconciliation and leave session creation disabled.

## Verification

`machine_cli::auth_watch` tests exercise native filesystem notifications,
atomic symlink replacement, unrelated file traffic, directory recreation and
connection cancellation. `machine_plugins::auth_watch` checks the bounded path
plan. `machine_plugins::auth_refresh` checks isolated probe success, refusal and
timeout. `noncanonical_refresh_survives_until_the_service_cas_wins` checks that
a missing sibling cannot suppress a rotation and that the next Service bundle
repairs all projections without touching native state.

The ignored `native_claude_refresh_probe_conformance` accepts an explicit
immutable CLI path in `COWBOY_TEST_AUTH_PROBE_CLI`. It uses only synthetic
credentials in a disposable home and does not log in or run inference. Run it
when accepting a new native CLI's authentication probe semantics.

Release verification must separately establish the installed watcher count,
Service generation advancement, agreement of credential projections and native
session recovery. A green isolated test is not evidence of those live effects.

## Hawk acceptance, 2026-09-17

Code revision `60ab09c0f47bf840288e369b95d661430fde8e9c` passed
`just check-compact`, final Rust Clippy/tests, and the native CLI conformance
test. The final Rust run passed 1,429 tests; the standalone Machine passed 340.
The complete gate also passed 1,730 Web tests and the disposable PostgreSQL
tests. An additional network-isolated native probe left both future-expiry and
expired synthetic credential files unchanged.

The Machine component receipt recorded `outcome=succeeded`, `phase=committed`
at 08:23:25 UTC. The release is
`/nix/store/x94byxm885i96dsw5rmcxb9v0pn0hpg5-cowboy-machine-release`; the ACP
generation remained `worker-c7f0635a1884fdd1ac6c`.

The new watcher installed 217 directory watches, rising to 220 when the Service
advanced Claude authentication from generation 5 to 6. All six runtime
projections matched the new immutable Service materialization. The invalid
cleared-token snapshots were rejected, and the surviving valid rotation was
promoted without another login. Native `auth status --json` returned success
and `loggedIn=true` for both previously failed projections.

One pre-existing crashed worker required a scoped restart after its crash
diagnostic had been lost; its original native session resumed and the Controller
recorded `recovery_outcome=session_running` at 08:25:59 UTC. This change does not
alter crash-diagnostic retention. `/healthz` and `/version` returned HTTP 200.
The next natural eight-hour upstream refresh was not part of this immediate
postflight; atomic replacement after watch installation is covered by the
native filesystem regression test.
