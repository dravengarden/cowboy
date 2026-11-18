# cowboy-account-sandbox

Root daemon that runs each Antigravity account's processes as that account's
own static system identity (`cga-<key>`) in confined transient systemd units.
Design: [`docs/antigravity-real-accounts-design.md`](../../docs/antigravity-real-accounts-design.md)
("Sandbox daemon", "Account directory layout", "Generations and login",
"Account units", "Kill switch and logout" and the Revision 4 amendments).

This crate is the daemon only. Machine and worker integration, the Node proxy
and the OVH host configuration (socket unit, service unit, config file,
allow-list) live elsewhere.

## Build and test

Standalone crate with its own `Cargo.lock`, built as a fully static musl binary:

```bash
nix develop -c bash -c 'cd packaging/account-sandbox && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check'
nix build .#cowboy-account-sandbox
```

Unit tests run unprivileged against a fake host (`src/fake.rs`) for user
management, images, mounts and systemd. `tests/exec_confined.rs` exercises the
real Landlock wrapper and skips itself on kernels below ABI 6.

## Commands

```text
cowboy-account-sandbox serve [--config PATH] [--socket PATH]
cowboy-account-sandbox exec-confined -- PROGRAM [ARGS...]
cowboy-account-sandbox --version
```

- `serve` takes its listening socket from systemd socket activation
  (`LISTEN_PID`/`LISTEN_FDS=1`, descriptor 3); `--socket PATH` binds one itself
  (mode 0660) for development. It must run as root, reconciles all persisted
  state on every activation and then serves until stopped. The socket's owner,
  group and mode are the socket unit's (`SocketUser=root`, `SocketGroup=ubuntu`,
  `SocketMode=0660` on OVH); the daemon assumes no group name.
- `exec-confined` is every unit's `ExecStart` wrapper (amendment R3-3): it sets
  `PR_SET_NO_NEW_PRIVS`, applies a Landlock ruleset with no handled filesystem
  or network access and `scoped = ABSTRACT_UNIX_SOCKET | SIGNAL`, then execs.
  It refuses to exec on kernels below Landlock ABI 6.

## Configuration

One root-owned JSON file, default `/etc/cowboy-account-sandbox/config.json`
(a missing default file means all defaults; an explicit `--config` must exist).
Every field is optional; unknown fields are rejected.

| Field | Default | Meaning |
|---|---|---|
| `state_root` | `/var/lib/cowboy-account-sandbox` | Account trees and runtime store |
| `run_dir` | `/run/cowboy-account-sandbox` | Per-unit descriptor files and execution sockets |
| `agy_allow_list` | `/etc/cowboy-account-sandbox/agy-sha512.allow` | One lowercase hex SHA-512 per line; `#` comments |
| `exec_path` | `/usr/local/libexec/cowboy-account-sandbox` | Installed binary used as the units' `exec-confined` wrapper; must be world-executable |
| `home_image_bytes` | `2147483648` (2 GiB) | Size of each sparse ext4 home image |
| `home_image_inodes` | `131072` | Fixed inode count of each image |
| `free_space_reserve_bytes` | `8589934592` (8 GiB) | Free space required on `state_root` before `ensure_account` and every start |
| `ip_deny` | `["10.0.0.0/8","172.16.0.0/12","192.168.0.0/16","100.64.0.0/10"]` | Extra `IPAddressDeny=` entries; `localhost link-local multicast` are always denied. OVH adds its public address, e.g. `40.160.141.233/32` |
| `ip_allow` | `["127.0.0.53/32"]` | `IPAddressAllow=` (the resolver stub) |
| `max_runtime_units` | `1` | `runtime`/`probe` units per account (plus at most one `login`/`logout` unit, fixed). One runtime per account bounds it to one turn; a second start is `limit` (design "Revision 5 simplification") |
| `unit_memory_max` | `"3G"` | `MemoryMax=` per unit |
| `unit_tasks_max` | `512` | `TasksMax=` per unit |
| `unit_tmp_size` | `"256M"` | Size of each unit's `/tmp` and of its separate `/var/tmp` (both tmpfs) |
| `max_runtime_archive_bytes` | `1073741824` | Largest runtime archive and unpacked content |
| `max_runtime_entries` | `50000` | Most entries per runtime archive |
| `max_runtimes` | `6` | Runtimes kept in the store (runtimes of live units are never evicted) |
| `stop_timeout_secs` | `30` | Wait for a stopped unit to disappear; also `TimeoutStopSec=` |
| `tools.systemd_run` | `/usr/bin/systemd-run` | |
| `tools.systemctl` | `/usr/bin/systemctl` | |
| `tools.useradd` | `/usr/sbin/useradd` | |
| `tools.userdel` | `/usr/sbin/userdel` | |
| `tools.mkfs_ext4` | `/usr/sbin/mkfs.ext4` | |
| `tools.mount` | `/usr/bin/mount` | `-o loop` attaches the image |
| `tools.umount` | `/usr/bin/umount` | |

## Protocol `cowboy-account-sandbox-v1`

Newline-delimited JSON, sequential requests per connection, one response line
each: `{"ok":true,"result":{...}}` or
`{"ok":false,"error":{"code":C,"message":M}}` with `C` one of
`invalid_request`, `not_found`, `fenced`, `conflict`, `limit`, `precondition`,
`insufficient_space`, `runtime_rejected`, `internal`. Request lines are at most
64 KiB; unknown fields are refused. Ids are non-empty printable ASCII of at most
256 bytes; keys are 16 lowercase hex characters.

| `op` | Fields | Result |
|---|---|---|
| `ensure_account` | `service_id`, `account_id` | `key`, `user` |
| `register_runtime` | `sha256`, `size`, then exactly `size` raw bytes | `sha256`, `already_present` |
| `probe_runtime` | `runtime_sha256` | `runtime_sha256`, `unit`, `exit_code`, `passed` |
| `start` | `key`, `mode` (`runtime`/`login`/`logout`/`probe`), `credential_generation`, `runtime_sha256`, optional `session_id`, `descriptor_b64` (≤ 32 KiB decoded) | `unit`, `mode`, `invocation_id`, `boot_id`, `fds` |
| `stop_unit` | `unit` | `unit`, `stopped` |
| `stop_account` | `key`, optional `revision` + `operation_id` (together) | `key`, `fence_epoch`, `fenced`, `stopped`, `revision`, `replayed`, `account_present` |
| `admit_account` | `key`, optional `revision` + `operation_id` (together) | `key`, `fence_epoch`, `fenced`, `stopped`, `revision`, `replayed`, `account_present` |
| `list_units` | `key` | `units` (each with `alive`, `owned`), `unrecorded`, `credential`, `fenced`, `boot_id` |
| `unit_state` | `unit` | `recorded`, `state`, `alive`, `owned`, `invocation_id`, `boot_id`, `current_boot_id`; unrecorded units add `requires_reconciliation` |
| `logout_cleanup` | `key`, `credential_generation`, optional `operation_id` | `credential_generation`, `token_removed`, `credential_cleared`, `history_retained`, `superseded`, `operation_id` |
| `remove_account` | `key`, optional `operation_id` | `removed`, `home_removed`, `user_removed`, `operation_id`, `replayed` |

`key = hex(sha256("cowboy-account-v1" || 0x00 || service_id || 0x00 || account_id))[..16]`.

`probe_runtime` runs the registered runtime's fixed `probe` command
(`bin/agy-launch --version`) in a confined unit of the daemon-owned probe
identity (`service_id` `cowboy-account-sandbox`, `account_id`
`runtime-probe`), waits up to 30 s for it, then stops and confirms the unit
gone before answering. It never touches a real account or credential; the
Machine runs it once per Provider generation before that generation's first
account unit, so no vendor code of an isolated Provider runs as the Machine
user. `ensure_account` refuses the reserved `service_id`.

A successful `start` response carries, as `SCM_RIGHTS` on its line, the
caller's ends of the unit's stdin (write), stdout (read) and stderr (read)
pipes, plus for `runtime` the daemon-created listening execution socket
(`<run_dir>/units/<nonce>/exec.sock`, owner the caller, group `cga-<key>`,
mode 0660, bound into the unit at `/run/cowboy-exec.sock`). The `fds` array
names them in order. `invocation_id` is `null` only when the unit finished
before its `InvocationID` could be observed (its output stays readable).
Closing the connection stops and confirms every unit it started. A stop that
fails keeps the unit's record; a sweep every 2 s (and every activation) retries
it until systemd confirms the unit gone. A failed `register_runtime` closes the
connection, because unread payload bytes may remain. Before any body byte is
read, a registration reserves twice its `size` (archive plus unpacked tree) on
top of every registration in flight above `free_space_reserve_bytes`
(`insufficient_space`); at most two registrations stage at once (`limit`).

A unit record is kept until systemd confirms the recorded invocation is no
longer active: the `systemd-run` client ending is not enough, and neither is
`ActiveState=inactive` alone: a unit with a queued job (`Job=` set) counts as
live, and only `LoadState=not-found` or an inactive/failed unit without a job
is gone. A systemd inspection error never confirms a stop. A recorded unit
whose `InvocationID` is unknown on either side is reported `alive` with
`requires_reconciliation: true`, never dead. `unit_state` of a unit systemd
runs without a record reports `alive: true` and `requires_reconciliation:
true`; `stop_unit` stops and confirms it.

When a start's `InvocationID` never appears, the daemon ends the
`systemd-run` client itself before it stops the unit, so the client can no
longer submit it after the stop is confirmed; until the client's exit is
observed the record stays `Starting` and is never confirmed stopped.

A login unit's outcome is the token's presence after a clean exit; an
inspection error leaves the record for the sweeper rather than failing the
login (no token is deleted on an error). The 2 s sweep also retries failed
partial-token cleanups. `logout_cleanup` syncs the token's directory after the
unlink before it reports the token removed.

### Runtime archives

A ustar archive (no PAX/GNU extension headers, links, devices or FIFOs, no
absolute or `..` paths) whose root holds `cowboy-runtime.json`:

```json
{"agy": "bin/agy", "hooks": "policy/hooks.json", "settings": "policy/settings.json"}
```

`agy` must be executable and its SHA-512 allow-listed (checked at registration
and again at every start). `hooks` and `settings` are bound read-only over
`~/.gemini/config/hooks.json` and `~/.gemini/antigravity-cli/settings.json`.
The per-mode commands are fixed in `src/units.rs` (`COMMANDS`):
`runtime` → `bin/agy-proxy`, `login` → `bin/agy-launch --cowboy-login`,
`logout` → `bin/agy-launch --cowboy-logout`, `probe` → `bin/agy-launch --version`.

## State

```text
<state_root>/                       root 0711
  fences/<key>.json                 root 0600 (fence tombstones)
  runtime/<sha256>/                 root, dirs 0755, files 0444/0555
  accounts/<key>/                   root 0711
    meta.json                       root 0600 (atomic writes)
    home.img                        root 0600 (sparse ext4)
    home/                           image mount; cga-<key> 0700 (HOME)
      .gemini/antigravity-cli/      cga-<key> 0700 (writable agy state, token)
    gemini/                         root 0755 (assembled ~/.gemini, bound read-only)
      config/hooks.json             placeholder bind target
      antigravity-cli/              bind target of the writable state
<run_dir>/units/<nonce>/            root:cga-<key> 0710 (descriptor, exec.sock)
```

Account-tree paths are opened with `openat2(RESOLVE_BENEATH |
RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV)` beneath the
state-root descriptor opened at start-up. The home mount point itself is opened
without `RESOLVE_NO_XDEV` (it is a mount), only after the daemon confirmed the
image is mounted; everything beneath it uses the full set again.

## Credential and fence rules

- `login` requires a credential generation newer than the recorded one and an
  absent token; it and `logout` (current generation only) first stop and
  confirm every runtime unit. While an authentication unit runs, `runtime` and
  `probe` starts are refused (`fenced`).
- A login unit that exits 0 with the token present makes the generation
  `signed_in`; anything else is `login_interrupted` and the partial token of
  that generation is removed (retried on later activations if it fails).
- `runtime` requires the current `signed_in` generation; older generations are
  refused.
- `stop_account` persists `fenced(epoch)` before stopping every unit and the
  account slice; only `logout`, `logout_cleanup` and `remove_account` run while
  fenced; `admit_account` clears it.
- Fence operations carrying the Controller's account-control `revision` and
  `operation_id` are ordered: the account persists the last applied
  disposition (`meta.json` `disposition`) and, under the account lock, refuses
  an older revision, or another operation at the applied revision, with
  `conflict`. Repeating the applied operation is `replayed` and changes
  nothing but re-confirms the stop. An unrevisioned `admit_account` is refused
  once a revisioned disposition exists; an unrevisioned stop is always applied.
- A fence of an account without metadata (never ensured, or removed) is
  persisted as the root-held tombstone `<state_root>/fences/<key>.json`
  (`account_present: false`); `ensure_account` adopts it before the account
  can start a unit, and `remove_account` leaves one with the account's fence
  state. `stop_account` therefore never answers `not_found`: its success is
  always durable fence evidence. Adoption is restart safe: `meta.json`
  records the tombstone it adopted (`adopted_tombstone`), so one a crash left
  behind is only removed, and a tombstone never replaces a disposition of a
  newer revision.
- `logout_cleanup` checks `credential_generation` under the same account lock
  as the token removal. An older generation is `superseded` (a later login
  required its token absent) and removes nothing; a newer one is
  `precondition`; repeating a completed cleanup of the current generation
  returns `token_removed: false`.
- `remove_account` requires the fence, a logged-out (or never signed-in)
  account and no token; it stops units, records the removal in the fence
  tombstone, unmounts and deletes the tree except `meta.json`, removes the
  user and deletes `meta.json` last, so a retry after a crash finishes the
  same removal. An account without metadata has no home: its units are
  stopped, a remaining user is removed and the removal is confirmed (a
  `replayed` one when the tombstone records the same `operation_id`). The
  account's keyed lock is never dropped, so no second lock of the account can
  exist while a request holds or waits on it.

## Known limits

- `settings.json` is bound over a path inside the account-writable state
  directory. Another live unit of the same account could race the bind target
  between the daemon's check and systemd's mount; the effect is limited to that
  account's own second denial layer.
- `/var/run` is masked separately only when it is a real directory; on hosts
  where it links to `/run` the `/run` mask covers it.
