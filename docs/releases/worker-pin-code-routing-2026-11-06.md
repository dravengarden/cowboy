# Worker pin advance for code-runtime routing — Hawk, 2026-11-06

## Why

Zed 1.22.0–1.24.0 were installed on Hawk, yet the code runtimes serving
worktrees were still 1.20.8 and 1.21.0 processes started a day earlier. A
worktree route kept the generation it first started with until a close left it
idle, and routes used only by code reads (`ensureWorktree`) are never closed.
The fix (`bc6efdbea`): an idle route re-selects the active generation when it is
next ensured or opened; a route holding any lease stays pinned. The tree-sitter
dependencies already on `main` had made the old pin (`82ff6af8`) refuse every
host-only Machine release, so the fix needed a pin advance.

## Acceptance of `bc6efdbea` before activation

All runs used disposable loopback fixtures and new receipt paths under
`/tmp/pin-acc2`, against `.#cowboy-machine-release` of `bc6efdbea`
(`z1ymqa1l…`):

| Gate                                | Input                                                                | Result                                 |
| ----------------------------------- | -------------------------------------------------------------------- | -------------------------------------- |
| `execution-worker-conformance`      | Codex plugin CLI 0.165.0 (`c06a67fd…`), release keeper               | accepted, 34 checks                    |
| `execution-worker-conformance`      | production executor `cowboy-execution-runtime` 0.159.3 (`8bf204b3…`) | accepted, 34 checks                    |
| `execution-session-conformance`     | `.#cowboy`, `.cowboy-machine-wrapped`, live Controller as recovery   | accepted, 11 checks                    |
| `agent-worker-conformance`          | Codex 3.5.21 (active generation `dfd8f100…`)                         | initialize, stop and drain passed      |
| `agent-worker-conformance`          | Claude Code 3.20.26 with 3.20.25 as previous                         | distinct generation coexistence passed |
| `execution-managed-conformance`     | Claude 2.1.314 from 3.20.26, production executor                     | accepted, 7 checks                     |
| `logs-conformance`                  | `bc6efdbea`                                                          | accepted, 11 checks                    |
| `code-buffer-connected-conformance` | live Controller `zzx26b78…`, installed Zed 1.24.0 pair               | accepted, 38 checks                    |

## Pin and activation

- Pin commit `d3a346c06`: `cowboy-workers` `82ff6af8` → `bc6efdbea`, worker
  generation `worker-4f248c87703274b63cd9`; interface files byte-identical to
  the pin at every later merge.
- Machine host release `7z2whfjz…` of `21bc16c34`, `--maintenance`: receipt
  `succeeded`, `committed`.
- Machine PID `1350862` → `1358659`; Controller PID `598697` unchanged; all 91
  execution keepers retained. The two local ACP workers were recycled into the
  new generation, as a pin advance does. The 1.20.8 and 1.21.0 Zed runtimes
  ended with the old Machine.

## Production result

`cowboy.code.diagnostics` on Go files no Code pane held acquired them for the
agent (`acquired: true`) through Zed 1.24.0 and reported per-server evidence:
`gopls` `unknown`, reason `unversioned`. Zed merges pulled diagnostics without a
document version, so they are not claimed as analysis of the exact text.
