# Remote session start latency, 2026-10-09

Investigation of the iPhone report that the `Mac Proxy tools` Claude session
(`sess-1791179743141`, runtime OVH, target Hawk) showed "You can start typing
while this session prepares…" for a long time. Its 10:51:56 UTC start spent
105,532 ms in `sdk-initialize`; an earlier start of the same session took
18,583 ms. The reconnect replay fix (`f2f28be0`) removed one 30 s retry wait;
this document covers the remaining stalls and the ordinary startup cost.

## Proven causes

### Shared single-flow transport saturated by bulk writes

- Every OVH session's execution request, reply, worker event and heartbeat
  travels in one Machine WebSocket, which is one TCP flow through Stormbird's
  overlay. Measured with an independent flow over the same path, OVH-to-Hawk
  throughput is about 100–120 KiB/s (4 MB in 32–39 s; Hawk-to-OVH 200–250 KiB/s).
- The stalls are machine-wide: between 10:52:00.6 and 10:52:22.1 no OVH session
  completed any execution call, then about a dozen sessions completed together.
  The OVH worker logs contain 572 such multi-session flushes (≥ 8 s, ≥ 3
  sessions) between 2026-10-06 and 2026-10-09.
- The overlay itself stayed usable during a Cowboy stall. From 12:18:01 to
  12:18:17 UTC thirteen sessions completed no call, while a long-lived SSH echo
  stream to OVH answered every second (184–1208 ms) and ICMP continued.
- Byte accounting of the Hawk loopback leg (caddy to Cowboy, 7 minutes, only
  per-type totals retained) attributes about 80 % of OVH-to-Hawk bytes to
  `fs/writeFile`, almost all 1.33–1.42 MB full hook-transcript copies from
  `sess-1791179743141`. That session still runs Claude 3.19.4, which copies the
  whole transcript to the target for each Bash PreToolUse hook; 3.19.5 and later
  send appends (10–19 KB in the same capture).
- Below the application, the overlay proxy buffered such a burst: its socket
  to the Controller held up to 1.27 MB unsent after the 12:40 reconnect and
  drained at about 90 KiB/s. Heartbeats overtook only the application queue, so
  every frame behind those bytes, including heartbeats, waited. When the
  Controller heard no broker frame for 35 s it dropped the connection
  (10:53:12); at 10:53:08 a Controller command still reached OVH in 160 ms.

### Serial startup discovery

The Claude launcher discovered target context with 83 calls in about 22
sequential round trips before native Claude started: `fs/getMetadata` for each
instruction, hook and MCP candidate (mostly absent), then `fs/readFile`, and
two to three calls for each of nine git and listing utilities. The 18.6 s start
spent its first ~11 s there at 500–1000 ms per call, then ~7 s in native
initialization, including target MCP servers (`npx -y chrome-devtools-mcp@latest`
alone takes 1.5–2.3 s on Hawk).

## Not established

- Which bulk frames were in flight during the original 10:52 stalls; no capture
  existed then. A Stormbird outbound failover toward OVH's endpoint at 10:52:20
  (and proxy dial failures at 10:52:09/10:52:20) shows additional path
  disruption at that moment, but fan-outs explain only 11 of 144 large stalls.
- Memory pressure at the time; a later sample cannot exclude it.

## Changes

- Claude 3.19.10: one ended target command (the startup survey) reports the
  platform, native's git status queries, rule and skill listings with sizes and
  candidate existence. Walks read only existing files with known metadata and
  run beside the instruction walk. Any missing utility, unexpected section,
  malformed row or newline in a listed tree falls back to the previous per-walk
  queries; later Reads never use survey answers. Startup milestones are logged
  as `[cowboy-claude] phase=… totalMs=…`.
- Machine transport `credit-v1` (negotiated by both ends; old peers keep the
  previous framing): chunk bytes beyond the receiver's decoded credit are
  bounded to 128 KiB, and small execution requests and replies overtake a
  chunked frame. Worker events, acknowledgements and all other frames keep their
  order. No credit progress for 60 s fails the connection.
- The Controller logs Invoke/Observe forwards slower than 2 s under the
  worker's operation identity, and both ends log how a Machine WebSocket ended.

## Measurements

Startup discovery on Hawk's real files and processes with injected per-call
latency, three runs each, identical rendered context:

| RTT | Before | After | Calls |
| --- | --- | --- | --- |
| 50 ms | 1.13 s | 0.21 s | 83 → 13 |
| 200 ms | 4.43 s | 0.81 s | 83 → 13 |
| 600 ms | 13.2 s | 2.4 s | 83 → 13 |

Transport on a simulated 100 KiB/s, 200 ms link with a 1.4 MiB transfer and a
small request every 200 ms: request delay p50 10.0 s / max 14.0 s unpaced,
p50 1.1 s / max 1.2 s paced.

Production `sdk-initialize` on OVH before the change (2026-10-06..09, all
targets and versions): new native sessions n=57, p50 14.3 s, p90 34.7 s, max
105.5 s; resumed n=493, p50 18.0 s, p90 48.6 s, max 149.8 s.

## Production rollout

From clean commit `af27fbec` on `origin/main`:

- Controller `/nix/store/nx34f7g9ivsvxrfk02gprzvg2l94d05h-cowboy-controller-release`
  committed 15:29:59 UTC; OVH, Hawk, Falcon and macbook-air reconnected with
  `paced=false`, as old Machines must.
- OVH Machine host-only maintenance
  (`/var/lib/columbus/ovh-machine-af27fbec71d7-20261009T153118Z`, release
  `/nix/store/994gm7ska9y0rwlizf3j043n44ccl32h-cowboy-machine-release`) was
  accepted with every worker retained; the Machine reconnected with
  `paced=true`.
- Claude 3.19.10 (artifact `sha256:44627ab18be0d7305d64200ea43555829d4a6c08d804fc13216d3efd55b11579`)
  passed the active, rollback and cold Catalog readers, was published, and
  `cowboy operator converge --machine ovh --plugin claude-code` upgraded OVH from
  3.19.9. Existing sessions keep their generation until reloaded.

Gates: `just check` (2061 core, 2148 web), `just claude-remote-check` (291),
packaged `execution-worker-conformance` (92 checks, full scope) and
`execution-session-conformance` (15 checks), six managed Codex review rounds
(the four survey findings are fixed with regression tests; the last round
reported none).

First 46 minutes after the OVH Machine reconnect, compared with 12:00–15:28
the same day (OVH worker logs, every execution call):

| | Before | After |
| --- | --- | --- |
| Calls / active sessions | 53,302 / 23 | 8,422 / 9 |
| Call p50 / p90 / p99 | 1.18 / 2.02 / 19.97 s | 0.63 / 1.92 / 6.00 s |
| Multi-session stalls (≥ 8 s, ≥ 3 sessions) | 57.1 per hour | 0 |
| Controller broker heartbeat timeouts | 2 at 15:17 alone | 0 |

The window is short and the load lower. `sess-1791179743141` (still 3.19.4)
was the busiest session afterwards, with 42 of its own calls over 5 s, which
are its full transcript copies; other sessions no longer stalled behind them.
After the running sessions moved to 3.19.10 (idle automatic updates and
reloads), `sdk-initialize` until 17:20 measured: resumed n=39, p50 7.4 s,
p90 9.5 s, max 15.2 s; new n=1, 5.0 s. Most samples are update reloads with
warm OVH caches, so they are not the same conditions as the earlier baseline. The Controller's new forward log aligned two
operations of that session: 2,046 ms at the Controller against 3,597 ms at the
worker, and 2,001 ms against 2,181 ms.

## Remaining

- Sessions on Claude 3.19.4 or earlier keep copying full hook transcripts and
  remain slow themselves until reloaded with a newer Provider.
- The per-flow overlay throughput (about 100 KiB/s) is a Stormbird path
  property; it is outside this change.
- Native initialization (about 5-7 s, including target MCP server startup)
  is unchanged. The SDK discarded the launcher's stderr, so 3.19.10 milestone
  lines never reached the worker log; 3.19.11 relays them through a private
  socket to the adapter's stderr.

## Readiness without the /cost turn (2026-10-10)

3.19.11 relayed the launcher's milestones to the worker log. 3.19.12 showed,
over five production starts: execution connection ~0.2 s, target discovery
~1.9 s, native spawn to the context module's receipt ~1.0 s, then 3.6–5.4 s
until the readiness `/cost` result, with the receipt arriving about 20 ms
before native's initialize reply. A local probe of Claude 2.1.287 explains the
wait: in SDK mode the first turn blocks until every `--mcp-config` server has
connected (30 s for a server that never answers; `MCP_CONNECTION_NONBLOCKING`
does not change it), and target MCP servers start through the Machine link.

3.19.12 replaced the second SDK initialize with the module's authenticated
`/ready` receipt. 3.19.13 reports ready at the initialize reply when that
receipt has arrived and sends `/cost` only otherwise; a missing module still
fails before any prompt reaches native. The first prompt still waits for
native's own MCP connection. First 3.19.13 starts: ready at 2.8 s, 3.6 s and
7.3 s (the last with 6.2 s of target discovery). Two new sessions at 03:49 and
03:50 UTC waited 18 s and 33 s for their execution connection while Hawk's
`stormbird-device` was being restarted by a separate rollout.

## Background task waits (3.19.14)

A native background task held on a target command read its output with a
1 s `process/read` wait for as long as the command ran: one Machine-link
call per second per running background command. It now reads without
waiting and sleeps on the executor's notification for that process (at
most 10 s), as target MCP relays already did (25 s). Pre-starting target MCP
servers before native was rejected: native passes `CLAUDE_CODE_SESSION_ID`
and `CLAUDECODE` when it starts a server, so an earlier start would differ
from a local session.

## Replacement snapshot during reset (Controller 3d8b93f5)

From 04:00 to 06:33 UTC on 2026-10-10, 7 of 12 isolated reload starts waited
6–30 s for `execution-connected` while every fresh start took 0.2–0.4 s. The
Controller marks a session `resetting` and drops worker `Snapshot` frames
until the reset `CommandAck`; that acknowledgement removes the worker entry.
When the replacement worker's snapshot arrived before the acknowledgement it
was discarded, so the replacement stayed unknown and its execution `Describe`
was refused as unavailable (retried every second) until a later periodic
snapshot. The Controller now stashes a replacement snapshot (different epoch,
not a broker placeholder, not exited) seen while resetting and adopts it when
the reset is acknowledged. Covered by
`replacement_snapshot_before_reset_acknowledgement_owns_the_session`.
