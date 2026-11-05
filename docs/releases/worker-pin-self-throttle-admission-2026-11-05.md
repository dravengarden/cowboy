# Worker pin advance for self-throttle admission — Hawk, 2026-11-05

## Why

A 25 GiB runaway script held at its execution unit's 24 GiB `memory.high` kept
host memory PSI "full" near 70 %. The Machine counted that local stall as Device
scarcity and reported zero launch headroom, so every worker launch (including
managed reviews) was refused while other sessions had ample memory. `82ff6af8`
treats a stall explained only by a unit at its own limit as local. Wire changes
already on `main` (`d33ee12a` memory relay, `5044d951` decoded-session
authorization) had made the old pin refuse every host-only Machine release.

## Source

- Fix `82ff6af8` on `main`; full `just check-compact` green.
- Pin commit `6eed6018`: `cowboy-workers` moved from `c4399ba4` to `82ff6af8`
  (worker generation `worker-ad21c529fda66b830581`).

## Acceptance of `82ff6af8` before activation

All runs used disposable loopback fixtures and a new receipt path:

| Gate | Input | Result |
|---|---|---|
| `execution-worker-conformance` | Codex plugin CLI 0.165.0 (`c06a67fd…`), release keeper | accepted |
| `execution-worker-conformance` | production executor `cowboy-execution-runtime` 0.159.3 (`8bf204b3…`) | accepted, 34 checks |
| `execution-session-conformance` | `.#cowboy`, `.cowboy-machine-wrapped`, previous Controller as recovery, `browser_device` | accepted, 11 checks |
| `agent-worker-conformance` | Codex 3.5.21 (active generation `dfd8f100…`) | initialize, stop and drain passed |
| `agent-worker-conformance` | Claude Code 3.20.26 with 3.20.25 as previous | distinct generation coexistence passed |
| `execution-managed-conformance` | Claude 2.1.314 from 3.20.26, production executor | accepted, 7 checks |
| `logs-conformance` | `82ff6af8` | accepted, 11 checks |
| `code-buffer-connected-conformance` | Controller `ahqvcspx…`, Machine `qj72dwx8…`, installed Zed pair `a88dcfb9…` | accepted, 38 checks ([receipt](../experiments/code-connected-worker-pin-2026-11-05.json)) |

The Claude managed gate needs the production executor: the packaged Claude
runtime pins `EXECUTOR_DIGEST`, and the Codex plugin CLI is refused as
`Unsupported execution descriptor`. Connected Code requires `/nix/store` native
inputs; the installed Zed pair was added to the store byte-for-byte (the
release's own adapter `1.20.12` differs from the installed generation). Session
deletion production conformance does not apply: Hawk runs reader-only
(`writerSchema 0`).

## Activation

| Lane | Release | Receipt |
|---|---|---|
| Machine | host release `w70y3aii…`, `--maintenance` | `succeeded`, `committed`, revision `6eed6018`, generation `worker-ad21c529…` |

- Machine PID `4002857` → `2704288`; Controller PID `2624364` unchanged.
- All 80 execution keepers retained. Of 3 local ACP workers, 2 kept their PIDs; the
  idle `sess-1791453646359` took a generation cutover and is running.
- Machine `ready`, `runtime_connected`, reporting 31.5 GiB memory headroom.
