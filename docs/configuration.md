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
| Device | `capacity.max_sessions` | `8` | restart |
| Device | `capacity.draining` | `false` | restart |
| Device | `disk.low_watermark` | `"15GiB"` | live |
| Device | `retention.artifact_cache_unreferenced_after` | `"7d"` | live |

`cowboy config explain --scope <scope>` is authoritative; this table is a
convenience.

## Disk retention

Disk growth from Plugin releases is retired automatically:

- Every `plugins.generation_retention_interval` the Service asks each Device
  to retire Plugin generations no recoverable session pins. In the same
  request, under the installation lock, the Device removes artifact-cache
  blobs that no retained generation references and that were not written
  within its `retention.artifact_cache_unreferenced_after`.
- Each Device reports its `disk.low_watermark` with its host resources. When
  its available disk falls below that watermark the Service runs an extra
  pass for that Device at once, then waits `plugins.urgent_retention_cooldown`
  before another. A Device without the setting (older release) or with
  `"0B"` never triggers one.

The budget is the Device's; the decision is the Service's, because only the
Service knows which generations sessions still pin.
