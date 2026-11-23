# Heartbeat-gap reproduction matrix (L0-b)

This is step L0-b of [host isolation](host-isolation-design.md#l0-requirements).
It tests which kinds of task load can stretch the gap between heartbeat
receipts beyond 45 s. The broker's isolation timeout is 45 s, so it serves as
the reference here. The test runs in a disposable NixOS VM and
changes nothing on the host.

## What it runs

The test files are in `tools/heartbeat-gap-repro/`:

| File | Role |
|---|---|
| `hbrepro.rs` | A std-only stand-in for the worker → broker heartbeat path |
| `vm.nix` | The guest: Hawk's slice shape scaled to 4 GiB, plus zram swap |
| `matrix.sh` | Runs inside the guest, one case at a time, and writes `summary.jsonl` |
| `run.sh` | Host driver: builds and boots the guest, runs the matrix, collects results, runs the isolation check |

How `hbrepro.rs` models the worker and broker:

- It uses the runtime wire's framing: a 4-byte big-endian length, then JSON.
- It copies the worker's shape: one writer is shared by a 10 s heartbeat and by
  bulk event frames.
- It allocates 1 MiB on each beat. This is a synthetic pre-send pressure
  probe, not a model of the worker. The real worker serializes a small
  heartbeat and sends it before it clones any pending outbox events
  (`src/worker.rs:812-814`). The probe shows whether `memory.high` throttling
  stalls an allocating sender in that slice.
- The worker reports these timings for each heartbeat:
  - wake lateness: the timer firing late;
  - allocation stall;
  - writer-lock wait: waiting behind a bulk frame;
  - send latency: from the beat's deadline to write completion.
- The broker reports the receive gap.

The slices in `vm.nix` follow Hawk's ratio (48G high and 64G max out of
93 GiB), scaled to the guest:

| Slice | Settings |
|---|---|
| `cowboy.slice/cowboy-agents.slice` | `MemoryHigh=2G`, `MemoryMax=2816M`, `MemorySwapMax=512M` |
| `cowboy.slice/cowboy-control.slice` | `CPUWeight=1000`, `IOWeight=1000`, `MemoryLow=256M` |

The guest runs the harness in the system manager. The cgroup v2 limits behave
the same way as under Hawk's user manager.

**Cases.** Each case runs for 180 s. Loads run in `cowboy-agents.slice`. The
`stress-ng --vm-bytes` value is a total across all workers, not per worker.

| Case | Worker slice | Load |
|---|---|---|
| baseline | agents | none |
| cpu | agents | `stress-ng --cpu 32` (8 × vCPUs) |
| memory-high | agents | `stress-ng --vm 4 --vm-bytes 2900M --vm-keep` |
| io | agents | `stress-ng --hdd 4 --hdd-bytes 256M` |
| combined | agents | cpu + memory + io |
| frame-block | agents | 32 MiB event frames every 1 s, broker reading 100 ms per 64 KiB (a deliberately stalled reader) |
| split-memory | **control** | memory load (the split remedy) |
| max-only | agents | memory load with `MemoryHigh=infinity`, `MemoryMax` kept (the policy remedy) |

**Run it:**

```bash
DUR=180 tools/heartbeat-gap-repro/run.sh
```

The script prints the results path. On Hawk, set `FLAKE` to the real path of
the Columbus `machines` flake. `run.sh` does this by default.

## Results, 2026-11-23

Run conditions:

- Guest: 4 vCPU and 4 GiB, using the hardened harness. Every case was valid:
  all units stayed active through a window that started after startup and the
  counter baseline.
- Hawk load average was 15 at the start and 27 at the end.
- The isolation check reported a host change. It came from another session's
  Hawk machine transaction, which reloaded the `nixos-fw` nftables chains;
  `/var/lib/hawk-deployments/current.json` was written at 07:11:43, inside
  the run. A user-network guest cannot create host nftables chains.

Each row is a single run of 180 s, so the remedy rows are single
observations.

| Case | Heartbeats | Max receive gap (ms) | > 45 s | Max send latency (ms) | Overdue at wake (ms) | Alloc stall (ms) | Lock wait (ms) | Worker memory `full` stall (ms) | Slice `memory.high` events |
|---|---|---|---|---|---|---|---|---|---|
| baseline | 18 | 10,000 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| cpu | 18 | 10,005 | 0 | 11 | 0 | 0 | 0 | 0 | 0 |
| memory-high | 18 | **16,836** | 0 | **6,860** | 0 | 1,300 | 0 | **8,556** | 23,837 |
| io | 18 | 10,073 | 0 | 5 | 0 | 3 | 0 | 0 | 0 |
| combined | 18 | 12,711 | 0 | 2,711 | 0 | 1,263 | 0 | **5,647** | 312,880 |
| frame-block | 15 | **51,602** | **2** | **46,124** | 36,124 | 0 | **46,124** | 0 | 0 |
| split-memory | 18 | 10,000 | 0 | 0 | 0 | 0 | 0 | **0** | 21,447 |
| max-only | 18 | 10,016 | 0 | 24 | 0 | 24 | 0 | 54 | 0 |

How to read the columns:

- **Max receive gap** includes silence before the first and after the last
  heartbeat in the window.
- **Send latency** runs from a beat's deadline to its write completion.
- **Overdue at wake** applies to beats that were already late when the loop
  reached them, behind an earlier stall. Their timer lateness is not counted
  as wake lateness, which stayed at most 10 ms in every case.
- Two earlier runs, before the window and teardown fixes, showed the same
  pattern, each with a 51.6 s frame-block gap.

### What the results show

- **Shared `MemoryHigh` throttling stalls the heartbeat sender.**
  - In `memory-high` and `combined` the worker shares the throttled slice. It
    spent 5.6-8.6 s of the 180 s fully stalled on memory.
  - A beat's deadline-to-write latency reached 6.9 s, and the largest
    receive gap was 16.8 s.
  - At 4 GiB with these loads, this never reached 45 s.
- **Split run (one observation).** In `split-memory` the same load drove the
  task slice over `MemoryHigh` 21,447 times. The worker in
  `cowboy-control.slice` recorded zero memory stall and zero send latency.
- **Policy run (one observation).** In `max-only`, with no shared
  `MemoryHigh`, the worker recorded 54 ms of stall and 24 ms of send
  latency.
- Neither single run establishes how well a remedy works; that decision is
  L0-7's, from Hawk's own records.
- **CPU and I/O alone added at most 11 ms** at these weights and sizes.
- **Only frame blocking crosses 45 s between receipts.**
  - In `frame-block` the heartbeat waited 46 s for the writer lock behind
    32 MiB frames that the broker read deliberately slowly.
  - That produced two closed gaps above 45 s, with a 51.6 s maximum.
  - This is a receive-gap measurement, not an isolation. The real broker
    refreshes liveness on any decoded frame, defers isolation while bytes
    are queued, and checks only every 15 s.
  - Isolation conclusions belong to L0's sweep records on Hawk.

### Limits

- **A stand-in, not the real broker.** The harness does not run the changed
  Cowboy broker, so it cannot show what L0 records under each load.
  - L0's records will come from Hawk itself (L0-7).
  - The sampler's own cost was measured separately on Hawk's live cgroups:
    6.5 ms per 10 s tick over 145 scopes. See the
    [L0 implementation](host-isolation-design.md#l0-implementation) section.
- **Scaled, not identical.** Hawk's pressure profile differs: 93 GiB,
  hundreds of tasks, real build and test loads. A 4 GiB guest cannot show
  that memory throttling alone reaches 45 s on Hawk. It only shows that the
  mechanism stalls the sender, and that the split or the policy remedy
  removes it.
- **Single runs under a busy host.** One 180 s run per case, while other
  sessions loaded Hawk, so small differences are not significant.

These results inform the L0-7 decision. They do not replace it:

| Hawk's gap records show | Remedy |
|---|---|
| Memory pressure on the worker's ancestors | Start with the policy remedy (`max-only`) |
| A frame blocked in its payload phase (`gap_frame_phase`, `gap_slowest_frame_*`) | L-lane |
