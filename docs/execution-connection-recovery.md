# Execution connection recovery

## October 8 Claude execution endpoint recovery

At 14:42:10 UTC, `sess-1791173104587` exhausted the worker's 180-second
execution response deadline and its private endpoint disconnected. At 15:44,
after Machine heartbeats had recovered, the same native Claude process still
rejected Read with `Execution unavailable; no replay` and refused Bash because
its target PreToolUse hook could not run. The Plugin's `Connection.closed`
flag was permanent: new calls never attempted to reconnect.

Claude Plugin 3.19.2 reconnects only for a new call, authenticates against the
original private descriptor, and initializes with the original executor
session ID. Concurrent callers share one handshake. A changed executor
identity is refused; explicit shutdown remains terminal. Calls whose replies
were lost still fail without replay, including writes and process starts.
Codex retains its native executor-session recovery; this change belongs to
Claude's existing Mods adapter and adds no Columbus session state.

The WebSocket regression commits a write and drops its acknowledgement, then
checks concurrent new reads, one reconnect, unchanged executor identity and
exactly one write. It fails on the previous Plugin. Packaged acceptance also
closes the real worker endpoint and exercises Read and Bash again inside the
same native Claude process, with the runtime filesystem untouched.

Official tool and Mods contracts were checked on 2026-10-08:
[tools](https://code.claude.com/docs/en/tools-reference),
[events](https://code.claude.com/docs/en/plugins/mods/events), and
[versioned types](https://code.claude.com/docs/en/plugins/mods/create).
CLI 2.1.287, ACP 0.84.0 and executor 0.159.3 remain pinned. No tool inventory,
schema or native hook disposition changes; only the existing adapter's
connection lifetime changes. Installed idle-session convergence and retained
busy generations must be reported separately from package publication.

The October 7 incident combined Machine connection replacement with a native
execution readiness failure. The Create dialog exposed the connection fence;
several Claude sessions encountered the installation reconciliation fence;
a Codex Remote session retained a failed target environment.

## Evidence and limits

At 20:32:25 and 20:34:59 Asia/Shanghai, the Controller recorded OVH's 45-second
Machine heartbeat deadline, followed by reconnection approximately two seconds
later. Broker heartbeat failures also occurred. The first Claude 3.14.0 install
retained an unknown staging receipt after its connection-bound authority ended.
Later reconciliation and an independently recorded installation completed;
inspection now reports `requires_reconciliation=false`. Historical unknown
receipts are preserved, not rewritten as success.

The affected Codex target keeper was ready at 20:31:52, but its first observed
endpoint connection arrived at 20:32:31. Native execution recorded a fixed
10-second initialize-handshake timeout. The adapter had already treated
`environment/add` success as readiness, although that API registers lazily.
Subsequent turns therefore inherited the failed environment.

A contemporaneous OVH TCP observation showed approximately 469 ms RTT,
449 KiB retransmitted out of 8.3 MB sent, and roughly 1 Mbps delivery. Existing
Machine writers serialized whole WebSocket messages, including multi-MiB
session traffic and heartbeats, with a 15-second whole-message deadline.
Independent writer tasks alone cannot prevent a large frame from delaying
heartbeats on a slow connection. This establishes an application-level failure
mechanism; the observation does not identify the physical cause of packet loss
or prove which historical frame triggered each disconnect.

## Recovery boundaries

Machine peers negotiate `x-cowboy-machine-transport: chunks-v1` in their HTTPS
upgrade. Both endpoints must agree; older peers retain text framing. Large
messages use bounded 16 KiB binary chunks and one bounded reassembly buffer.
Only heartbeat and WebSocket control traffic can overtake these chunks.
Application messages retain FIFO ordering and the existing authenticated
Machine/worker contracts and durable codecs remain unchanged. Malformed,
oversized, interleaved or unnegotiated chunks close the connection.

Creation may recover Inventory, PrepareRuntime and Prepare against a replacement
Machine connection for a bounded period. It retains the original session and
preparation inputs, relying on the target's existing exact-identity observation.
It never retries arbitrary execution calls, closes, user turns or installations.
Old-connection receipts cannot authorize a new connection.

The private Codex launcher uses native `environment/info` to establish readiness
and recover the same bound environment before exposing initialization success
or forwarding thread/turn startup. `environment/status` only observes and is
not a recovery API. Readiness queries have bounded retries; the user request is
forwarded once, or receives an error without dispatch. A later request can retry
readiness without changing placement or native thread identity. Outer worker
startup deadlines still apply.

This is an adapter around the native Codex environment API, not an alternate
executor. Remove it when the pinned native/ACP path guarantees readiness and
same-environment recovery itself. Claude shares the Machine transport fix;
its Mods adapter and installation safety fences remain independently owned.

## Validation

Rust regressions cover actual WebSocket backpressure, heartbeat interleaving,
application FIFO, old-peer framing and invalid chunk rejection. Creation tests
cover same-identity recovery and stale receipt fencing. Launcher tests cover
lazy registration, transient and persistent failure, recovery on a later turn,
and single dispatch of user requests.

`tools/execution_handshake_recovery_probe.py` runs the pinned native executable
with isolated homes and a loopback byte relay. It delays the first genuine
initialize reply past 10 seconds, then requires a second connection, successful
thread creation and target-owned guidance. No model request or Service credential
is used. Native 0.159.3 with SHA-256
`8bf204b36a2f6dd0dab73aa2f639892e67ef9ac8befccb4a05b1496ebf25c479`
passed in 11.121 seconds with two connections. This is recovery evidence, not
a production deployment receipt.

## October 8 small-frame replay follow-up

OVH repeatedly reauthenticated while Controller logs reported both the
35-second broker watchdog and the 45-second Machine watchdog. The host service
had zero systemd restarts; worker recovery and execution-response timeouts
followed the connection losses. An established Machine socket had sent about
40 MB while its last received application bytes were over 100 seconds old.
These observations identify the affected transport, but do not attribute every
historical disconnect to one frame or establish the physical loss mechanism.

The chunk writer admitted urgent traffic only while splitting a large frame.
During a backlog of small application frames, new heartbeats waited behind the
entire replay, including small frames already moved to the writer's private
queue. A deterministic sink that injects a heartbeat during the first of 32
small writes reproduced that ordering failure on the deployed implementation.

The writer now checks for urgent traffic between small writes as well as
between chunks. Both drains are bounded; application frames retain FIFO order,
and a silent connection retains the existing watchdog deadlines. The regression
requires the heartbeat immediately after the in-progress first write and
checks every application frame's original order. This transport is shared by
Codex and Claude; it does not change their native execution or recovery APIs.

The first deployment still experienced watchdog failures under production
replay. A second deterministic test uses a separately scheduled heartbeat
producer and a ready sink: the private queue drained all 32 small frames before
the producer could run. Priority alone cannot help a heartbeat that has not
been enqueued. The small-frame path now yields after each write, matching the
existing chunk path. Both regressions are required: prompt producer scheduling
and urgent-message ordering. This follow-up does not extend watchdog deadlines
or treat bulk byte progress as a broker heartbeat.
