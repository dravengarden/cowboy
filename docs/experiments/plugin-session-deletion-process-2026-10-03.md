# Session deletion process-crash acceptance — October 3

The private writer candidate now has actual OS-process crash/reopen acceptance
using temporary datasets and real Unix IPC. This is a source-test slice. The
production writer remains disabled, release admission is unchanged, and no
Machine, Controller or Web activation is required for these test-only hooks.
No production dataset, Provider runtime or live Session is used by the fixtures.

## Process matrix

Three parent tests launch 16 child broker processes from the same freshly
compiled Rust test executable. Thirteen are SIGKILLed and reaped; three exit
on the expected startup refusal. Output pumps are joined after exit, and all
waits have deadlines. The ignored child test is invoked only by its parent
fixtures with command-scoped fixture environment variables.

- Four isolated writer processes stop after staging write, file sync, atomic
  rename or directory sync. SIGKILL followed by IPC EOF confirms that no ACK
  was buffered. A newly launched reader observes no committed record before
  rename and never replays the retained staging file. After rename it reads
  the terminal record. The latter proves process-reopen visibility, not
  power-loss durability or a successful deletion ACK.
- A positive deletion ACK precedes SIGKILL of the writer. The terminal record
  survives two independently launched reader processes that are also killed,
  and another reader after two rejected foreign-owner attempts.
- A directory at the committed-file destination forces a persistence failure.
  The writer returns the specific negative deletion ACK and rejects subsequent
  admission as reader-unavailable. After SIGKILL, a fresh reader refuses this
  invalid committed entry before binding its broker socket.
- Terminal readers reject old workers before Welcome and reject both ordinary
  and adoption-only EnsureSession requests with the deletion-specific reason.
  They do not create worktrees. Uncommitted staging permits an ordinary worker
  Welcome. Machine and Service identity mismatches refuse startup with their
  specific owner error.

The checkpoint enum, callback field, setters and write-boundary calls all use
`cfg(test)`. The nested process test module is also compiled only for tests.
There is no production CLI, request or environment writer switch. The journal
write order and production reader constructor are unchanged.

## Verification

Run from the repository root in its pinned Nix development shell, removing
`COWBOY_PROVIDER_PACKAGE_PATH` from test commands so fixtures use their private
Provider stores:

```sh
env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --locked --no-default-features --features machine-host --lib
env -u COWBOY_PROVIDER_PACKAGE_PATH cargo test --locked --all-features
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo fmt --check
```

After integrating current remote main, the Machine-host library suite passed
459 tests with five ignored. The all-feature unit suite passed 1,766 tests
with 42 ignored; enabled integration
and doc-test targets also passed. The three process parent tests run in both
suites. Clippy, format and diff-whitespace checks passed.

## Remaining boundary

The executable is a source test artifact with a private writer enabled only in
its disposable fixture. It is not a separately supplied immutable old/new
release binary matrix. Power loss, older host activators, portable updater
recovery and general Machine recovery admission remain separate. These results
do not admit a production writer, native resume, continuous Session incarnation
or worktree ownership. The historical
[reader-floor release](../releases/plugin-session-deletion-floor-2026-10-03.md)
records the deployed owner boundaries; the
[dataset contract](../plugin-session-deletion-journal.md) records current scope.
