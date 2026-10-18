# Configuration

Cowboy owns its configuration. The Service (Controller) and every Device
(Machine) each read one independent TOML file through the same framework
(`src/config.rs`). Settings that need no UI are changed by editing that file;
nothing about them belongs in Columbus host configuration or the database.

## Files

| Scope | File | Read by |
| --- | --- | --- |
| Service | `<data-dir>/config/cowboy-service.toml` (Hawk: `/var/lib/cowboy/config/`) | `cowboy serve` |
| Device | `<state-dir>/config/cowboy-device.toml` (OVH: `…/services/svc-…/config/`) | `cowboy-machine` |

The files live in Cowboy's own writable state, not in a Nix store or a host
repository, so an agent can change them without a deployment. A missing file
means "all defaults"; existing deployments need no change. `config/history/`
keeps the previous 50 files written by `cowboy config apply`.

Commented templates listing every setting are generated from the schema:
[`config/cowboy-service.example.toml`](../config/cowboy-service.example.toml) and
[`config/cowboy-device.example.toml`](../config/cowboy-device.example.toml).
A test fails when they drift from the code.

## Commands

`cowboy config` exists in every Cowboy build, including Device releases.
Every subcommand accepts `--format json` with stable fields.

| Command | Purpose |
| --- | --- |
| `cowboy config check --scope S [--file F]` | Validate without changing anything; exit 1 on errors |
| `cowboy config show --scope S` | Effective values and whether each came from the file or the default |
| `cowboy config explain --scope S [KEY]` | Meaning, accepted values, default and reload class |
| `cowboy config schema --scope S` | Machine-readable schema of every setting |
| `cowboy config diff --scope S --candidate F` | What a candidate changes and which changes need a restart |
| `cowboy config apply --scope S --candidate F` | Validate, keep history, atomically install |
| `cowboy config init --scope S [--print]` | Write or print the commented template |
| `cowboy config path --scope S` | The file that would be used |

The Service file is located with `--data-dir` (default `/var/lib/cowboy`,
env `COWBOY_DATA_DIR`), the Device file with `--state-dir`
(env `COWBOY_MACHINE_STATE_DIR`), or either with `--file`.

Diagnostics name the dotted key, line and column, what was expected and found,
and a concrete fix: unknown keys get a "did you mean" suggestion, and every
value error states the accepted range and the default. Codes are stable:
`syntax`, `missing-schema`, `unsupported-schema`, `unknown-key`, `wrong-type`,
`out-of-range`, `invalid-value`, `not-a-regular-file`, `file-too-large`,
`unreadable`.

## Applying changes

Each setting declares when it takes effect:

- **live**: the running process polls its file every few seconds. A valid
  edit applies within seconds; an invalid edit is logged and ignored, so the
  running values never change because of a typo.
- **restart**: read once at startup. The running process logs that the change
  waits for the next restart.

Startup refuses an invalid file and prints the full report, before any state
is created. Together with `check` this gives the rolling-update rule: a release
or configuration change that replaces a running instance first runs the
candidate binary's `cowboy config check` against the configuration it will
read, and replaces the old instance only when it passes. A new release must
also accept the current file; a schema change ships an explicit migration.

Command-line flags and environment variables are wiring inputs. Where a legacy
flag overlaps a setting during migration (for example `--max-sessions`), the
explicit flag still wins and the setting's documentation says so.

## Where a setting belongs

Decide in this order; the first match wins.

1. **Identity, wiring or secret** (IDs, enrollment, URLs, sockets, paths,
   database URL, credentials): command-line or environment input supplied by
   the host. Never the configuration file. A configuration value may reference
   a secret by path or ID, never contain it.
2. **Created or changed through the product UI or API at runtime**, owned by a
   user, session, project or Machine record, needing concurrent edits,
   per-user permission or synchronization to clients (for example project
   placement policy, session options, accounts): the **database**.
3. **Operator-owned behaviour that needs no UI** (limits, thresholds,
   intervals, retention, feature switches, tuning): the **configuration file**.

Then choose the scope: a setting lives with the process that enforces it.

- **Device**: facts and budgets of that host and its local data, such as
  session capacity, memory and disk thresholds, local log and cache retention,
  disabled Providers.
- **Service**: fleet-wide behaviour and decisions across Devices, such as
  scheduling and reclaim choices, Plugin generation retention, usage refresh,
  authentication and telemetry policy.

A behaviour that spans both is split, never duplicated: the Device declares
its budget, the Service decides what to do when a budget is exceeded.

Choose `live` unless the value is consumed only while starting (sockets,
thread pools, capacity advertised at connect).

## Adding a setting

### Workspace storage paths

The Machine accepts repeatable
`--workspace-storage-root /absolute/source=/absolute/storage` path wiring.
Both directories must already exist. After mounting the intended storage, the
host provisions a regular, storage-owner-owned `.cowboy-storage` file containing
exactly `cowboy-workspace-storage-v1` and a newline. Cowboy never creates this
marker: an empty mountpoint is not sufficient startup admission. The host
must also gate startup on its mounts (system services can use
`RequiresMountsFor=`; user services need a mount precheck and retry).
New sessions for that source (including
source aliases and subdirectories) use `storage/worktrees/sessions`. Existing
sessions keep their previous root; all configured roots remain discoverable
and trusted for workspace adapters and bounded deleted-session Cargo cleanup.
Overlapping storage destinations, repeated sources and destinations inside a
source checkout or default worktree root are rejected. A missing or replaced
storage root fails closed instead of falling back to the default disk.

New execution environments and local ACP workers receive these cache paths:

| Variable | Value |
| --- | --- |
| `CARGO_BUILD_BUILD_DIR` | `{workspace-root}/target` |
| `GOCACHE` | `<storage>/cache/go` |
| `BUN_INSTALL_CACHE_DIR` | `<storage>/cache/bun` |
| `TMPDIR` | `<storage>/tmp` |

Cargo keeps each worktree's intermediates inside its marked `target` directory,
so bounded session-deletion cleanup reclaims them with the final outputs. Its
workspace template also keeps independent build locks and incremental state.
Existing caller-owned checkouts must already be on the intended disk; this
option does not relocate them.
Go and Bun share their project cache. Provider homes, credentials and shared
toolchain installations keep their existing ownership. Already running keepers
retain their original launch contract; this option does not move active
worktrees or rewrite admitted environments. Commands outside Cowboy keep their
host defaults unless the caller supplies these variables.

This is operator-owned host path wiring, so it deliberately uses CLI discovery
(`cowboy-machine --help`) rather than a UI/MCP setting. Applying a change needs
the host's Machine maintenance boundary.

### Behaviour settings

1. Append a `Field` to `SERVICE_FIELDS` or `DEVICE_FIELDS` in
   `src/config/schema.rs`: dotted key inside a section, kind with bounds,
   default, reload class and documentation.
2. Add a typed `Key` constant beside it and read it with `Config::get` (startup)
   or `Handle::get` (live). Re-read live values where they are used rather than
   caching them.
3. Regenerate the template:
   `cowboy config init --scope <scope> --print > config/cowboy-<scope>.example.toml`.
4. Add the key to the typed-key test in `src/config/tests.rs`.

Never add a new behavioural environment variable or a separate JSON policy
file. Existing ones move into these files one at a time, keeping the old input
as a deprecated override for one release.

## Current settings

| Scope | Key | Default | Reload |
| --- | --- | --- | --- |
| Service | `plugins.generation_retention_interval` | `"6h"` | live |
| Service | `plugins.urgent_retention_cooldown` | `"1h"` | live |
| Service | `plugins.catalog_keep_releases` | `3` | live |
| Service | `plugins.repin_dormant_sessions` | `false` | live |
| Service | `plugins.repin_dormant_after` | `"7d"` | live |
| Service | `plugins.auto_update_idle_sessions` | `false` | live |
| Service | `plugins.auto_update_idle_after` | `"1h"` | live |
| Service | `plugins.auto_update_idle_limit` | `"12h"` | live |
| Service | `sessions.reclaim_on_capacity` | `false` | live |
| Service | `sessions.reclaim_min_idle` | `"1h"` | live |
| Device | `capacity.max_sessions` | `8` | restart |
| Device | `capacity.draining` | `false` | restart |
| Device | `disk.low_watermark` | `"15GiB"` | live |
| Device | `retention.artifact_cache_unreferenced_after` | `"7d"` | live |
| Device | `file_buffers.enabled` | `true` | live |
| Device | `file_buffers.max_file_size` | `"2MiB"` | live |
| Device | `file_buffers.excluded` | secrets, media, notebooks, databases | live |
| Device | `file_buffers.cache_max_bytes` | `"64MiB"` | live |
| Device | `file_buffers.cache_max_age` | `"1d"` | live |
| Device | `file_buffers.target_temp_max_age` | `"1h"` | live |

`cowboy config explain --scope <scope>` is authoritative; this table is a
convenience.

## Dormant session re-pinning

Generation retention keeps every Provider generation a recoverable session
pins, so one forgotten session can hold an old release indefinitely. Every
scheduled retention pass records which sessions are dormant (exited, no worker,
nothing in flight, not a system session) in one persisted setting, so dormancy
is measured across Controller restarts; opening a session clears it.

Off by default. With `plugins.repin_dormant_sessions = true`, a session dormant
for `plugins.repin_dormant_after` is moved to its Device's installed release
through exactly the gate of an explicit Reload: the Device is connected, a
native session is saved, and the authentication and native session contracts
are unchanged. Nothing is started; the binding change is persisted, and the
next open resumes the native session on the new release. The same retention
pass can then retire the old generation. A session that fails the gate keeps
its generation.

## Idle session Provider updates

Re-pinning covers sessions without a worker. A session that keeps an idle
worker still holds its generation, so with frequent releases the retained
generations grow with the number of long-lived sessions. Off by default; with
`plugins.auto_update_idle_sessions = true`, every running, idle, non-system
session with no event for `plugins.auto_update_idle_after` moves to its
Device's newer installed release. It uses the existing per-session automatic
update path: the explicit Reload gate (connected Device, saved native session,
unchanged authentication and native contract), a recheck of the exact binding
and idle state under the Hub lock, native resume, and never a downgrade or
pre-release. A session that opted in individually still updates as soon as it
is idle. A dormant (exited, workerless) session under the same policy is only
re-pinned after the idle period, exactly like `plugins.repin_dormant_sessions`
but without its longer wait, so its next open starts the new release and no
Device slot is spent. Updates are paced: each 30-second pass starts at most one per Device,
and none while another session on that Device is still starting (one that has
been starting for over 10 minutes is logged and no longer waited for), so a whole
idle fleet never relaunches at once. An unattended update never replaces a
worker that reports background work, and when one leaves a session crashed,
further unattended updates to that release pause on the Device for 30 minutes
(an explicit Reload or update request still applies), and a session whose own
unattended update crashed is left alone until it is used or opened. Every
update logs `Provider update finished` or `Provider update left the session
crashed` with its duration.

A session nobody has used for `plugins.auto_update_idle_limit` is not kept
current in the background: restarting its worker on every release would resume
the native session many times for nobody. Its offer reports `on_open`, and
opening it (any client selecting the session, including after a reconnect)
records the same one-shot request as "update when idle", so it adopts the
installed release within seconds, before the first prompt. Workerless sessions
are still re-pinned regardless of the limit. Together with re-pinning, a
generation is retained only while a recently active session uses it.

Each pass also publishes, per session, the newer compatible release it could
adopt (`provider_update_available` on the session list) even while a turn is
running. Clients offer it in place: Desktop shows an `Update` top-bar action
(`␣TP`), Mobile badges the session Options button and shows an update card in
the session sheet. Updating an idle session reloads it now; a busy session
records a persisted one-shot request (`PUT /api/sessions/{id}/reload` with
`{"when_idle": true}`) that the pass applies within seconds of the session
becoming idle, and a dormant session is re-pinned without starting. A request
whose release is no longer newer is dropped rather than firing later.

## Session reclaim on capacity

Capacity (`capacity.max_sessions`) counts workers and admitted cold launches.
Creating a Web session prepares its isolated workspace and durable record;
it stays workerless until a message is sent or the user explicitly starts it
with Reload. Both direct and split placements can create records on a full Machine.
Workspace preparation, dormant history and workerless crashed records do not
consume worker slots. Legacy caller-owned and system creation retain their
eager startup behavior.

Cold prompts reserve admission under the same per-Machine decision fence as
memory reclaim and Provider maintenance. The Controller checks the worker
limit, draining flag and recent memory observation before dispatch. Messages
waiting for capacity remain in the existing durable queue, with a persisted
reason and original wait timestamp. Queue pause is also persisted, so a
Controller restart cannot resume deliberately paused work. Editing and
cancellation keep their ordinary semantics. Waiting queues resume oldest first within each
retry pass; a fresh interactive submission can arrive between passes. Crashed
or interrupted work requires its ordinary explicit recovery.

With `sessions.reclaim_on_capacity = true`, a waiting message on a full
Device allows the Service to hibernate its longest-idle eligible session,
wait up to 20 seconds for release, and count again (at most two reclaims per
pass). Disabling the policy leaves messages waiting until a slot is freed.
A session is eligible only when it holds
a slot, is idle (no turn, queued prompt or background task), is not a system
session, does not have Provider cache protection enabled, and has had no event
for `sessions.reclaim_min_idle`.

Opening a hibernated session shows its history without waking it: the
transcript lives in the Controller, so a dormant session costs its Device
nothing until it is used. The first prompt wakes it (the client shows
Starting for the whole launch) after admission. A waiting indicator explains
why it has not started. Recent wakes are protected for ten minutes. The
existing post-wake rebalance also covers legacy eager startup paths.

Settings remain available before the first message and throughout hibernation.
The focused bootstrap projects the exact signed Provider generation's defaults
and presets without starting a worker. Matching Machine, account and generation
snapshots can supply option labels and choices; they never supply another
session's selected values or add undeclared option ids. Providers
without a published preset/default contract expose their native options after
their first run. Existing sessions retain their own last advertised options.
The preview is not an ACP acknowledgement and is not persisted as one. Only
the selected preferences are durable; the worker validates and replays them
before the first prompt. Opening settings, editing preferences and clearing a
dormant conversation require no Machine connection or worker slot. Clear keeps
settings and the workspace. Compact performs model work and therefore wakes
the session through ordinary admission. Inspecting a failed session preserves
its failure until a prompt or explicit recovery is requested.

Desktop and Mobile share Settings → Resources & automatic maintenance.
Each Machine has independent switches for low-memory reclaim, capacity
reclaim and automatic idle Provider updates. UI overrides are persisted in
the database; unspecified fields inherit the Service TOML defaults. The
inventory is `GET /api/session-maintenance`; an operator changes only the
supplied fields with `PUT /api/machines/{id}/maintenance`. Background
Provider reloads yield to foreground resource waiters and memory pressure.
Dormant updates only change the retained Provider pin.

## Memory reclaim

Off by default. With `sessions.memory_reclaim = true`, the Service keeps each
Device's Agent workers below the cgroup limit that contains them. A Device
reports `memory_headroom_bytes` with every heartbeat (every 15 seconds): for
the worker slice and each ancestor, it takes the smaller finite
`memory.high`/`memory.max`, subtracts usage after discounting inactive file
cache, and floors the result at zero. It then takes the minimum across these
limits and host `MemAvailable`. This ordering does not overstate headroom
when usage already exceeds a limit. Inactive cache remains an estimate.
If host or ancestor memory PSI `full avg10` reaches 1%, the reported admission
budget becomes zero until pressure subsides. The UI labels it as an estimate,
separately from physical available memory.

When the headroom falls below `sessions.memory_reclaim_below` (1 GiB), the
Service hibernates the longest-idle eligible session on that Device, waits
until its worker has released and a report taken afterwards has arrived, and
repeats until the headroom reaches `sessions.memory_reclaim_until` (2 GiB).
Eligibility is that of capacity reclaim with its own idle threshold,
`sessions.memory_reclaim_min_idle` (15 minutes), and excludes sessions woken
in the last ten minutes. With nothing eligible it logs once every ten minutes
and waits. The Controller serializes cold admission with reclaim and reserves
512 MiB for each starting worker beside a 1 GiB minimum observation; a report
older than 60 seconds pauses cold admission. The Machine checks live headroom
(two at a time, 512 MiB reserved per launch beside a 512 MiB floor). A launch
that finds no headroom within 60 seconds is refused as resource exhaustion
rather than started, so a wake or create on a Device whose sessions are all
busy fails visibly instead of stalling the Device. A Controller-side wait
does not start a model turn or repeatedly transfer the queued message.

Hibernation sends no model request and the wake makes none before the
prompt (`docs/hibernation-token-audit-2026-10-06.md`). That is not proof that
the next prompt costs the same as after a warm idle: a Provider prompt cache
is kept by the Provider, not by the process, and survives a hibernation only
while it is unexpired and the resumed request repeats the same prefix. Cache
lifetimes differ by Provider and billing (for Claude Code, one hour on
included subscription usage and five minutes otherwise), and a relaunch can
change the prefix, so a threshold below the lifetime can turn a cache read
into a cache write. Idle time counts from a session's newest activity: prompts, agent
output, tool calls and permissions. Lifecycle edges and the command, mode,
configuration and usage updates a worker replays when it starts or reconnects
do not count. A Controller restart restores the clock from the newest such
durable event, so a deploy does not restart the wait for reclaim or idle
Provider updates; a session without one counts as freshly active.

## Disk retention

Disk growth from Plugin releases is retired automatically:

- Every `plugins.generation_retention_interval` the Service asks each Device
  to retire Plugin generations no recoverable session pins. The Service also
  keeps every generation a worker it currently observes was launched with,
  and the Device keeps every generation a resident Session there was launched
  with (its launch registry and each live worker's self-reported launch),
  whatever the Service pins: a worker generation cutover relaunches each
  registered Session on its exact generation, and a surviving worker is
  reconciled with the session record only on reconnect. A Device without a
  running broker refuses the request rather than guessing. In the same
  request, under the installation lock, the Device removes artifact-cache
  blobs that no retained generation references and that were not written
  within its `retention.artifact_cache_unreferenced_after`.
- A session whose Device refuses to launch its pinned generation because that
  generation is not installed there (retired before these guards, or pinned
  on another Device) is moved to the installed release by the Provider update
  pass within seconds, through the same gate as an explicit Reload and
  regardless of the update policy; a queued prompt is delivered by reviving
  it. A session that fails the gate keeps its generation and needs a Reset or
  Reload, and the Service logs why.
- Each Device reports its `disk.low_watermark` with its host resources. When
  its available disk falls below that watermark the Service runs an extra
  pass for that Device at once, then waits `plugins.urgent_retention_cooldown`
  before another. A Device without the setting (older release) or with
  `"0B"` never triggers one.

- In the same scheduled pass the Service retires its own Catalog releases
  that nothing resolves any more: no recoverable session pins them, no Device
  reports them installed or targets them in an install operation, the host
  policy does not select them, and they are not among the newest
  `plugins.catalog_keep_releases` versions of their Plugin. A release or
  artifact directory younger than a day is never retired. Every Catalog read
  root is swept, and an artifact directory goes only when no retained release
  in any root still names it. `0` keeps every published release.

The budget is the Device's; the decision is the Service's, because only the
Service knows which generations sessions still pin.

## Remote file buffers

The `file_buffers.*` settings decide which files a remote session keeps a
verified local copy of, so an Edit or Write sends only a hash-checked splice
to the target. They also bound how long and how much of those copies, and of
the target's leftover temporary files, is kept. The Machine projects them to
`<state>/file-buffers.json` for running sessions. See
[`remote-file-buffers.md`](remote-file-buffers.md).
