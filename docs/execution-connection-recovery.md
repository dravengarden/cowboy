# Execution connection recovery

## October 8 tool latency follow-up

The Controller still recorded OVH Machine and broker heartbeat timeouts after
the small-frame fairness change, including 16:11 and 16:21 UTC. Filtering only
lines containing both `ovh` and a selected timeout phrase missed broker records;
that filter is not evidence of an outage-free observation window.

At 16:14:10 UTC multiple independent executor sessions completed requests that
had waited about 18 seconds, within milliseconds of one another. The shared
Machine link preserves application FIFO, so a large message delays unrelated
tool requests and replies even when heartbeat scheduling is fair. Claude's
target hook path then sent a full native transcript for each hook invocation.
A live 1,942,094-byte transcript produced 2,589,478 bytes after base64 wrapping.
An independent OVH-to-Hawk SSH transfer took 9.832 seconds for 1 MiB, compared
with 2.535 and 2.721 seconds for empty SSH requests. Stormbird additionally
reported proxy UDP response timeouts; these are separate underlay evidence,
not proof that every slow tool call has the same physical cause.

Machine peers now separately negotiate `x-cowboy-machine-compression: zlib-v1`
alongside `chunks-v1`. Large messages compress only when this reduces their
size by at least ten percent. A high bit on the connection-local chunk length
identifies compressed payloads; the existing authenticated application messages,
worker protocol and durable records are unchanged. Compression uses an isolated
dictionary for each message; compression and decompression run off the async
executor. Both encoded and
decoded payloads retain the existing frame limit; malformed, truncated,
trailing, unnegotiated and oversized streams fail closed. Old peers keep their
existing text or chunked transport. Heartbeats still overtake chunks, while
application frames remain in order.

Fast zlib reduced that real base64 envelope to 1,003,942 bytes (61% fewer), in
41.24 milliseconds. This is a measured payload reduction, not a claimed 61%
end-to-end speedup. Regression tests reconstruct all application bytes, check
the following tool reply's order and reduced wire budget, and exercise real
WebSocket backpressure with compression both enabled and disabled.

The [accepted observation](experiments/machine-tool-latency-compression-2026-10-08.json)
links the protected host-maintenance receipt and records production negotiation
at 17:10:21 UTC. All 26 workers survived the host-only switch. Over the following
6.5 minutes, 2,663 execution RPCs begun after activation had a 170.6 ms median,
352.1 ms P95 and 3,874.3 ms maximum; no heartbeat or execution-response timeout
appeared in that observation. RPC duration excludes model generation and is not
whole-tool duration. This finite observation does not prove the underlying
proxy path will remain loss-free.

## Incremental hook transcripts

Claude Plugin 3.19.3 replaces repeated large transcript uploads with verified
appends. Native Claude still owns history on the runtime Machine; project hooks
still receive a complete, private snapshot on the execution Machine. For
transcripts of at least 128 KiB on a target with Cowboy's file helper, the adapter retains one
latest base in memory and one private cache file per execution binding (each
bounded by the existing 8 MiB transcript limit). No transcript content enters
durable adapter state or telemetry. Retained older keepers without the helper
and smaller inputs keep the bounded full-copy behavior. Python is not used by
the 3.19.6 adapter for this path, range reads or symlink resolution.

Only an exact byte prefix permits an append. The target verifies both the base
and assembled SHA-256 digests before exposing an exclusive per-hook snapshot;
hooks never receive the cache path. Snapshot preparation is serialized, while
hook commands can still run concurrently. Truncation, compaction and rewrites
send a new full base. A missing, changed or symlinked cache explicitly requests
a full refresh before any hook runs. Transport errors never retry a process
start; the next distinct hook starts with a fresh base after uncertainty.
Each invocation removes its temporary inputs and snapshot. The single cache is
disposable, replaced atomically, and contains only the latest bounded copy.

Source regression exercises a multi-megabyte Unicode/CRLF transcript, concurrent
snapshots, zero-byte unchanged uploads, cache damage, rewrites and a lost
preparation receipt. Packaged-native acceptance additionally requires repeated
target hooks to read the original large prompt after subsequent history appends.
This optimization reduces transcript transfer, not model requests or the
underlying network's round-trip time.

The [3.19.3 release receipt](experiments/claude-hook-transcript-release-2026-10-09.json)
records all 92 packaged-native checks, 252 source tests, Catalog/worker
compatibility, public artifact readback and completed OVH installation. The
synthetic 2,300,000-byte fixture uploads 23 bytes on its next append and zero
body bytes when unchanged; these exclude control framing. Existing running
sessions keep their generation until the normal idle upgrade, so installation
alone is not evidence of lower latency in an already running native process.

Codex owns execution semantics and resume; this host-to-host transport gap is
outside the native runtime. Claude shares the same transport, with no separate
tool or session implementation. Delete this framing extension when the owning
Machine transport provides bounded compression and heartbeat isolation itself.

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

The [release receipt](experiments/claude-execution-recovery-release-2026-10-08.json)
records the exact packaged 91-check acceptance, current/rollback Catalog
readers, old/new worker coexistence, public artifact hashes and completed OVH
installation. Version 3.19.2 is active on OVH. Existing sessions retain their
generation until the configured one-hour idle upgrade gate permits replacement.
The original session resumed with the same native ID and completed `echo ok`
on retained 3.19.1; this production recovery is distinct from the packaged
3.19.2 same-process reconnection proof. Busy sessions were not interrupted.

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

## Explicit execution lifetime recovery

An upgraded Machine does not replace the keepers of existing sessions. Keepers
from before the completed-operation tombstone eviction fix can exhaust their
65,536-entry ledger and refuse subsequent operations with `capacity`, including
launcher initialization. An established transport and a quick target refusal
distinguish this failure from a network watchdog timeout. Updating the Provider
alone does not update these already-running execution processes.

The runtime worker is another independent release boundary. OVH's retained
`worker-748825b42b4302fe26ca` (source `660a714b`) predates `464818d9`, which
normalizes native parameterless `params: null` requests to the execution wire's
required object. That worker can connect successfully and then prevent Codex
initialization from completing. Its replacement requires explicit worker-pool
maintenance, not merely a host-only Machine update. Preserve busy workers
through the host transaction, then use the owned generation handoff or scoped
Session recovery to replace them. Never change a Session's workspace or native
conversation to hide a worker-version mismatch.

Host-delegated `cowboy operator execution-sessions` lists bound sessions.
`cowboy operator recover-execution --session ID` returns a plan and exact request.
Save its `data.request` object, then apply it with
`cowboy operator recover-execution --session ID --request FILE`. Only the local
Operator socket exposes this maintenance API; runtime and target Machine protocol 27 is
required. Active turns/background work refuse unless the saved request explicitly
sets `interrupt_active_turn: true`. Interruption stops commands; uncertain effects
must be inspected, never automatically replayed.

The Service first commits a non-runnable recovery intent. The target records the
exact operation, closes the old lifetime, archives its state, and starts a new
incarnation with the same native executor and worktree. Native conversation,
Provider, credentials and queued messages stay in their existing Session record.
An explicit worker recovery command resumes that conversation after the new
binding is committed; ordinary worker adoption still cannot change placement.
Queued messages pause in the current Controller. Unacknowledged prompts are
retained as durable unscheduled drafts for inspection, never automatically
replayed after another Controller restart. Codex and Claude share this Machine lifecycle; each keeps its own
native conversation and Provider adapter. This layer owns the cross-Machine
process replacement gap, not native execution. Remove it when native runtime
recovery can safely replace and attest the bound remote process itself.

After a lost response, repeat only the saved request. Its operation identity and
binding revision prevent creating a second lifetime. A recorded but unobservably
started target remains fenced; recovery never clears a native start marker to
guess that replay is safe. Older readers preserve the opaque recovery intent and
refuse launch. A successful target receipt is not proof of Provider readiness:
observe the resumed Session and target connectivity before reporting success.

### October 9, 2026 production recovery

Controller `d2e5e752` and the OVH Machine were activated with successful owner
receipts. Hawk activated writer-host `f7236e0c`, retaining both schema-one writer
capabilities, with a same-revision, same-generation reader recovery artifact.
The selected worker generation is `worker-db4bc66cf4d579fbffa6`.

All 26 OVH-to-Hawk bound Sessions reached `running`; all 26 target keepers and
their 26 OVH workers were observed on the accepted binaries. The 23 vulnerable
keepers were replaced. The other three Sessions also needed worker recovery
after their exited/crashed runtime was observed. Workspace paths and directory
device/inode identities were unchanged. All 22 nonempty native conversations
retained their IDs. Three previously cleared Claude Sessions and one empty
Codex Session acquired fresh native IDs; their event records contained no user
messages or tool history. Recovery did not submit synthetic user prompts.

Exact artifacts passed 15 Session recovery checks, 11 keeper checks, 34 Codex
worker checks and 86 packaged Claude worker checks. The complete repository
gate also passed. After the maintenance window, the Controller observation
contained no new capacity, watchdog, timeout or session error. This observation
establishes startup/readiness recovery, not a guarantee against future network
faults. Queues remain paused for inspection; uncertain prompts remain drafts.

Sanitized artifact and activation evidence is recorded in
[`experiments/remote-execution-recovery-2026-10-09.json`](experiments/remote-execution-recovery-2026-10-09.json).

### Large hook transcripts and restricted target PATH

A later `OVH auto cleaner` tool attempt exposed a separate defect: its native
transcript was 7,339,309 bytes. The target's service PATH did not contain Python,
so the optional incremental helper fell back to one full `fs/writeFile`.
Base64 alone expanded that request to 9,785,748 bytes, above the worker's
7,340,032-byte invocation limit. Validation rejected it before target admission;
the PreToolUse guard correctly refused the Bash call but reported only that the
hook could not run. Startup readiness did not exercise this large-input path.

Claude Plugin 3.19.5 bounds transcript and hook-input uploads to 3 MiB raw chunks before
assembling a private target file. Both first/cache-miss copies and the no-Python
fallback use that path. Assembly has a unique temporary path, cancellation on
unknown results and cleanup; it never retries a hook. Python discovery also
checks the standard Linux and NixOS system locations when PATH omits them, so
Hawk can use verified append-only synchronization without changing user PATH.
The wire limit and the project's hook decision remain unchanged.

Regression coverage includes 8 MiB snapshots with and without Python and
over 6 MiB of genuine native history accumulated across ordinary-sized turns
in the packaged hook scenario. That history exceeds the invocation limit after
Base64 encoding and must still reach target hooks intact. Each UserPromptSubmit
guard also receives its complete input. A separate fresh native executor probe
checks an 8 MiB hook input without exceeding Claude's per-turn context budget.
Native Claude/ACP pins and public tool/Mods
contracts are unchanged; only private transcript transport and helper discovery
change.

The [release and recovery receipt](experiments/claude-hook-frame-recovery-2026-10-09.json)
separates package acceptance, OVH installation and the affected session's adoption.

### Cowboy-owned target file utilities

The 3.19.6 adapter removes its Python programs. The execution keeper's own Rust
binary implements bounded snapshot assembly, whole-file-stamped range reads
and symlink resolution. The keeper supplies its immutable absolute executable
path as `COWBOY_EXECUTION_FILE_HELPER` in the owned executor environment; this is
private executable wiring, not a behavioral setting or operator PATH lookup.
The reserved Cowboy environment namespace cannot be supplied through an
operator's target environment configuration. No helper is uploaded into a
project or downloaded on demand.

Native Codex still supplies filesystem RPCs, process execution, cancellation
and remote environment binding. Its current filesystem interface lacks the
verified append/snapshot and whole-file-stamped range operations this adapter
needs. The narrow extension is an ordinary native `process/start` invocation
of the owned utility, with the existing keeper ledger and permissions. No new
RPC, execution authority, Provider-specific keeper state or replay mechanism
is introduced. Remove these utilities when the native interface supplies the
same bounded operations and integrity guarantees. Claude is adapter-backed;
Codex's native tool contract remains unchanged.

Deploy the updated target keeper and recover its execution lifetime before
expecting incremental behavior from existing bindings. An older retained
keeper remains compatible through bounded full copies and ordinary file reads;
it does not regain Python dependencies. Projects may still explicitly use
Python in their own commands or hooks. Development and acceptance fixtures
also use Python; neither is an implicit Cowboy production runtime dependency.

The [owned utility deployment receipt](experiments/cowboy-owned-file-helper-2026-10-09.json)
records native acceptance without an interpreter on PATH, Hawk activation and
OVH installation. The affected session's keeper has been recovered with its
native identity and worktree preserved; adoption of the new Provider remains
separate from installation and must be verified per session.
