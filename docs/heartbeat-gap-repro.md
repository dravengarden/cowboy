# Heartbeat-gap reproduction matrix (L0-b)

This is step L0-b of [host isolation](host-isolation-design.md#l0-requirements).
It tests which kinds of task load can delay a worker heartbeat past the
broker's 45 s isolation timeout. The test runs in a disposable NixOS VM and
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
- It allocates 1 MiB on each beat, as the real worker does when serializing.
  So `memory.high` throttling hits the sender exactly where it would hit the
  real worker.
- The worker reports three timings for each heartbeat:
  - send lateness: the timer firing late;
  - allocation stall;
  - writer-lock wait: waiting behind a bulk frame.
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

- Guest: 4 vCPU and 4 GiB.
- Hawk load average was 124 at the start and 12 at the end. Worker-pin
  acceptance builds were running at the same time.
- The `vm.sh isolation check` reported the host network unchanged.

| Case | Heartbeats | Max gap (ms) | > 45 s | Worker memory `full` stall (ms) | Alloc stall (ms) | Lock wait (ms) | Slice `memory.high` events |
|---|---|---|---|---|---|---|---|
| baseline | 18 | 10,001 | 0 | 0 | 1 | 0 | 0 |
| cpu | 18 | 10,008 | 0 | 0 | 0 | 0 | 0 |
| memory-high | 18 | 10,191 | 0 | **2,001** | 779 | 0 | 29,575 |
| io | 18 | 10,000 | 0 | 0 | 0 | 0 | 0 |
| combined | 18 | **12,455** | 0 | **6,236** | 1,271 | 0 | 361,187 |
| frame-block | 15 | **51,606** | **2** | 0 | 0 | **46,142** | 0 |
| split-memory | 18 | 10,000 | 0 | **0** | 0 | 0 | 22,469 |
| max-only | 18 | 10,033 | 0 | 56 | 33 | 5 | 0 |

### What the results show

- **Shared `MemoryHigh` throttling stalls the heartbeat sender.** In
  `memory-high` and `combined`, the worker shares the throttled slice. It
  spent 2-6 s of the 180 s fully stalled on memory, and its per-beat
  allocation stalled for up to 1.3 s. At 4 GiB with these loads, the stalls
  delayed a heartbeat by 2.5 s at most. They never approached 45 s.
- **The split removes the stall.** In `split-memory` the same load drove the
  task slice over `MemoryHigh` 22,469 times. The worker in
  `cowboy-control.slice` recorded zero memory stall.
- **The policy remedy almost removes it.** In `max-only`, with no shared
  `MemoryHigh`, the stall dropped to 56 ms.
- **CPU and I/O alone did not delay the heartbeat** at these weights and
  sizes.
- **Only frame blocking crosses 45 s.** In `frame-block` the heartbeat waited
  46 s for the writer lock behind a 32 MiB frame that the broker was reading
  slowly. That produced two gaps of 51.6 s, close to the 48-58 s seen on Hawk.
  The broker's read delay was set on purpose, so this shows that the
  mechanism can reach that size. It does not show that it happened on Hawk.

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
- **Silence fields came later.** Later runs also report:
  - silence before the first heartbeat and after the last one
    (`leading_silence_ms`, `trailing_silence_ms`, `max_silence_ms`);
  - the worker unit's state (`worker_state`).

  The run above predates those fields. Every case except `frame-block` saw
  all 18 expected heartbeats.

These results inform the L0-7 decision. They do not replace it:

| Hawk's gap records show | Remedy |
|---|---|
| Memory pressure on the worker's ancestors | Start with the policy remedy (`max-only`) |
| A frame blocked in its payload phase (`gap_frame_phase`, `gap_slowest_frame_*`) | L-lane |
