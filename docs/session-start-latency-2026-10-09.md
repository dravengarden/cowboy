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
