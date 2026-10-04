# Restore an absent package from the original floor proof — October 4

The preceding recovery command restored missing selection pointers only while
the accepted anchor package remained intact. An absent package or entire lost
component cache could not be reconstructed from locally retained original proof.

Linux Machine-host installers now accept paired local evidence inputs:

```sh
cowboy-machine-install --restore-floor-selection \
  --state-dir /absolute/machine/state \
  --artifact-public-key /absolute/publisher.pub \
  --anchor-manifest /absolute/original-manifest.json \
  --anchor-artifact /absolute/original-artifact
```

The existing private floor must bind the canonical state, publisher and uncommitted
deletion dataset. A closed manifest (including ID, reader and optional probe)
must declare reader 1/writer 0. Its version, generation, artifact digest and exact
canonical proof hash must match the first accepted floor; the configured publisher
must verify its signature. This is the same signed selection, not authorization
for a replacement version. JSON formatting and the unsigned artifact URL do not
grant identity or selection authority; no URL is fetched and no probe is executed.
The authenticated supplied manifest/archive bytes are captured before staging.

Regular no-follow/nonblocking reads cap manifest at 64 KiB, key at 16 KiB and
artifact at 256 MiB. Raw packages leave entrypoint unset. Archive recovery accepts
only regular files/directories, a safe existing entrypoint, no duplicate/conflicting
files, at most 512 MiB exposed payloads, a decoded stream bounded to that limit plus
64 KiB, 65,536 tree entries, 4,096-byte paths and 128 components. Paths normalize
the existing archive's current-directory components but refuse traversal and
absolute prefixes. Permissions exclude special bits and group/other write access;
the selected executable is mode 0755. All unpacked bytes are verified against the
signed artifact again before publication.

A completely absent canonical generation is staged in a fresh mode-0700 private
directory beside its eventual destination. Regular files are created exclusively,
proof files mode 0600, and files/directories are synced. Floor bytes, publisher and
selection evidence are checked again before Linux atomic no-replace publication.
Existing intact generations authenticate idempotently; any damaged, partial, linked
or conflicting destination refuses without replacement. Staging failures retain
their unselected directories; this command performs no cleanup or global retention
management. Missing regular cache parents may be recreated after authentication.

Existing selection pointers must use the exact absolute original anchor targets;
other pointers or linked/non-directory parents refuse before staging. Thus exact
dangling pointers can resume after package publication, and an entirely absent
cache can be rebuilt before the existing pointer recovery runs. Interruption after
package publication is idempotently resumable; publication never exposes an
incomplete package. The two selection pointers remain separately synced and
fail-closed between updates. This is not a full power-loss recovery proof or a
same-user/admin mutation fence. Non-Linux package restoration refuses while the
preceding selection-only command remains available.

The command does not rewrite floor, bootstrap, identity, token or launcher, and
does not admit committed deletion records, rotate publisher keys, repair a damaged
generation, select current inventory or enable the deletion writer. Administrator
sudo rights remain intact. A local retained original signed package is required;
there is no automatic network recovery or migration.

## Verification and production receipt

The source gate passed before integration: all-feature Clippy, default Clippy,
1,814 all-feature tests, 503 standalone Machine-host tests and the Provider gate.

Exact native acceptance and activation evidence are appended after integration.
Incoming main changes common dependencies, so the retained-worker interface guard
must refuse a host-only release. Any full Machine candidate needs separate
worker/Code maintenance acceptance before activation; source checks alone do not
accept the new pool generation.


The integrated candidate is source `90ec4edae56349cb06b9f39f13a0086197b5356b`,
full immutable bundle
`/nix/store/rs21kbymnbgsdqv52dajg1wn0s2lvclq-cowboy-machine-release`,
worker generation `worker-dc63f38423cbd1971eda`. Independent maintenance
acceptance is recorded in
[the native receipt](../experiments/plugin-anchor-maintenance-2026-10-04.json):
Codex and Claude signed generation coexistence and descendant drain passed on the
exact native worker; native execution passed 22 and 33 checks respectively;
real public remote-session admission/recovery passed 15 checks; connected Code
passed 38 checks with cleanup complete and no failure. The worker, app-server
bridge and core Code adapter differ from the preceding retained bundle. Zed
adapter/server and JS runtime paths and digests remain equal. This finite local
acceptance does not claim external target coverage or live Provider upgrades.

The integrated gate passed 1,819 all-feature tests (50 explicit native ignores)
and 503 Machine-host tests (12 ignores), both Clippy configurations, formatting,
and the Provider gate. A first run's 100 ms OTLP fixture failed to enter its first
HTTP attempt under concurrent builds; the focused test and complete gate repeated
successfully without changing code or timeouts. The exact immutable installer
passed all 36 anchor-package cases, 28 selection-pointer cases, 5 signed refresh
cases, 9 signed startup cases, 24 cache-startup cases and the old-package bootstrap
negative control. These tests use disposable local proof and cache fixtures;
production floor and cache recovery were not invoked.

Only after this independent acceptance is the retained worker input advanced to
the accepted candidate. The pinned private source uses native authenticated SSH
fetching; GitHub's unauthenticated archive returned 404. No credential is copied
into an artifact. The final host-only artifact must retain all six accepted
companion paths and digests and carry the candidate's exact retained source.
Production activation evidence follows after that artifact is built and checked.


Production activation completed from clean published source
`76f53d0b33897d0929b121498da52e5ef52ae450` using host artifact
`/nix/store/fbvj2g8lja3qsvlh4l7f4iryzv1rlpdn-cowboy-machine-release`.
The installed Machine-owned activator remained
`/nix/store/a4jpv2prm56mvkcfy8f0w969hdm4srmc-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`
(SHA-256 `9e1294320f0242363dc426aa856b602c3e55816a250544334e2a353bae545f8f`).
Its independent root transaction `1791106864576617390-76f53d0b3389`
started at `2026-10-04T09:41:04.57661739Z` and committed successfully at
`2026-10-04T09:41:15.935862929Z`; published true, maintenance true,
recovered false. The receipt selects `worker-dc63f38423cbd1971eda` and preserves
all six companion paths and digests from the independently accepted source
`90ec4edae56349cb06b9f39f13a0086197b5356b`.

The bounded before/after observations, `09:40:33.259Z` and `09:41:37.838Z`,
are in [the production receipt](../experiments/plugin-anchor-package-recovery-2026-10-04.json).
Machine PID changed `706832` → `1434181`. All 13 detached worker and 5 keeper
unit IDs, active states and PIDs remained equal. Controller PID `812989` and its
component receipt remained equal; Web profile remained equal. `/healthz`,
`/version`, `/`, `/sw.js` and Hawk deployment health all returned HTTP 200;
the Hawk Machine reports online with the accepted default generation. Root reader
floor SHA-256 remains
`26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`;
the portable deletion namespace still contains only `.lock`, with no portable
floor initialized or production recovery command invoked. `/etc/sudoers`
SHA-256 remains
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`
and `sudo -n true` succeeds. No Controller/Web/OS activation or active-session
recycle belongs to this transaction. The Machine deletion writer remains closed.

Remaining boundaries: existing damaged generations still refuse; recovery needs
locally retained original signed evidence and a surviving floor. Publisher-key
rotation, committed deletion datasets, non-Linux package publication and writer
activation have no recovery admission in this slice.
