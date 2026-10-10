# OVH native execution and Controller communication efficiency

Status: research on 2026-10-04 against Cowboy `c0a999fe`. No production
configuration, worker, Plugin or service was changed. This extends the
[fleet and historical-task analysis](cowboy-ovh-efficiency-2026-10-04.md). The
goal is fewer bytes and fewer serial round trips while retaining exact target
binding, signed releases and mutation deduplication.

## Actual integration

The current installed OVH Providers are Codex 3.3.1 and Claude Code 3.4.3.
Installed versions alone do not identify every retained worker generation. A
fresh, restricted `/proc` read found both Providers' bridge processes with
runtime `ovh` and execution environment `hawk`. The
[sanitized readback](experiments/ovh-native-binding-readback-2026-10-04.json)
contains only Provider names, logical Machine IDs and process IDs. Multiple
bridge processes can belong to one session; this is not a session count. Native
children can deliberately scrub descriptor variables. Absence of that variable
is not evidence of local execution or missing credentials.

- **Codex:** native app-server on OVH, pinned 0.159.3. The adapter registers
  `environment/add`, then binds `thread/start` and `turn/start` to the exact
  execution environment and workspace. The target runs the pinned native
  `exec-server`, without Provider authentication. This is not a shell tool
  implemented by asking the model to generate SSH commands.
- **Claude:** native Claude Code 2.1.287 on OVH. Its Mods `tool.call` hook
  routes native Bash, Read, Write, Edit, Glob, Grep, NotebookEdit and TaskStop
  through the private bridge to the bound executor. Unsupported native surfaces
  fail explicitly; a failed bridge never falls back to OVH's local files or
  shell.
- **Transport:** local native bridge → OVH worker/broker → persistent
  authenticated Machine WSS → Cowboy Controller → bound target executor. Hawk
  execution uses its local broker; Falcon and Mac execution additionally use
  their existing enrolled Machine connections. New target permissions must not
  be inferred from a transport endpoint.

Code owners are `components/provider-runtime/packages/codex-acp/launch.mjs`,
`plugins/claude-code/runtime/`, `src/worker_execution.rs`,
`src/machine_control/execution.rs` and the Machine/broker transport. OpenAI's
[self-hosted environment documentation](https://developers.openai.com/api/docs/guides/agents-api/environments/self-hosted)
describes a separate cloud-harness integration; switching Cowboy to that API is
not required for these improvements. Anthropic documents the native
[Mods event surface](https://code.claude.com/docs/en/plugins/mods/events).

Ordinary target tools require logical workspace and Machine identity, not
physical IPs, geography, JMS endpoint knowledge or credentials. Provider API
egress and the target overlay are separate network decisions. This research does
not provide a fresh packet capture proving Provider direct egress or JMS
routing; those remain Stormbird-owned routing acceptance requirements.

## Ranked opportunities

| Priority | Change                                                                        | Evidence and expected benefit                                                                                                                                                                                   | Ownership                                            |
| -------- | ----------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| 1        | Avoid repeated full generation verification within one usage invocation       | Historical median 27.90 s preparation versus 1.30 s collector; repeated verification paths exist. Individual verification cost still needs profiling.                                                           | Machine Plugin host                                  |
| 1        | Target-side range Read and versioned conditional Edit                         | A 10-line read transfers an entire 1 MiB file, twice for two ranges. Reduce bytes and avoid metadata/read round trips while retaining a whole-file version stamp.                                               | Claude adapter and executor capability               |
| 1        | Longer event-woken process output waits                                       | Current adapter makes 60 output reads for a simulated quiet 60-second command. A supported 10-second wait would need about six quiet reads, without delaying output if the backend wakes on output/exit.        | Claude adapter and pinned executor                   |
| 1        | Bounded output/search processing at the target                                | Grep retrieves up to 64 KiB and applies pagination on OVH; later pages can rerun the same search and resend the prefix. Useful for large repos and multi-file diagnosis.                                        | Claude adapter and executor                          |
| 2        | Byte budgets, fair bulk scheduling and urgent-control capacity on Machine WSS | Controller and Machine writers use unbounded FIFO queues and send/flush individual frames. Large file/output frames share these queues with control traffic. Actual production queue delay is not measured yet. | Controller and Machine transport                     |
| 2        | Stage timing across the full request chain                                    | Existing local timing cannot attribute a delayed mobile send to OVH inference, preparation, Controller routing or bulk queueing. This is necessary to validate transport changes.                               | Web, Controller, Machine and Provider, independently |
| 2        | Reduce startup guidance RPCs using a target-owned snapshot                    | Claude checks ancestor instruction files using metadata then whole-file read, in batches of eight. A versioned snapshot can reduce serial rounds without losing canonical guidance.                             | Claude adapter and executor                          |
| 2        | Measure and consolidate Matrix turn preparation if costly                     | `observe` then `context` are serial and awaited before native turn input. Each has a 2 s timeout. Combined transactional preparation could avoid a round trip; current latency is not measured.                 | Matrix service and memory client                     |
| 3        | Batch independent read-only diagnostics and reduce repeated output            | Archived task history has repeated Git/read commands; classification includes intentional before/after checks. Real tasks should combine independent checks and bound output.                                   | Native agent workflow and project guidance           |

Priority 1 is a concrete implementation candidate, not a claim of a measured
post-change speedup. Benefits depend on workload: whole-file reads and long
quiet builds offer much more scope than reading a short README.

## Reproducible adapter measurements

The probe (in Git history) imports the
current `WorkspaceTools` implementation, supplies a generated remote-file
fixture and records its actual RPC methods. It also supplies a simulated clock
and quiet-process responses to the actual output collection loop. It uses the
pinned development shell, creates only its own temporary state, and cleans that state.
No inference, network fault or real business file is used. See
[results](experiments/ovh-native-efficiency-2026-10-04.json).

| Case                                      | Current behavior                                                                             |
| ----------------------------------------- | -------------------------------------------------------------------------------------------- |
| Read 10 lines from a 1 MiB generated file | `fs/getMetadata` then `fs/readFile`; 1,398,104 base64 bytes for 1,279 selected content bytes |
| Read another 10 lines from the same file  | Same two RPCs and another 1,398,104 bytes                                                    |
| Quiet command exits after simulated 60 s  | 60 `process/read` requests, each requesting a 1,000 ms wait                                  |

The file payload is about 1,093 times the selected text bytes, excluding JSON
envelopes, transport framing and duplicated native presentation fields. Range
reads cannot simply discard the existing whole-file hash: edits currently
require a matching prior read stamp. The target should return an authoritative
version/hash and apply conditional edits against that version. Preserve limits,
encoding, file type, conflict behavior and access restrictions. Native executor
capability must be verified before designing an unsupported RPC method.

Process output optimization must preserve sequence cursors, prompt output/exit
wakeups, immediate TaskStop admission and no mutation replay after lost
receipts. It is not safe to introduce an unconditional ten-second sleep. The
fixture is a request-count simulation, not measured WAN time or proof that a
proposed wait is supported by the pinned executor.

`worker_execution.rs` already uses a 20-second event long poll, with early wake
when events arrive. An otherwise quiet connected execution client therefore
needs roughly three empty event polls per minute, not sixty. This existing
stream is a lower priority than Claude's separate one-second process reads.

## Controller to OVH: what to optimize and what to retain

Machine WSS is persistent and bidirectional; replacing it with repeated HTTP
calls or adding another SSH tunnel would increase work. Current separate
reader/writer tasks deliberately prevent full-duplex deadlock. The transport
tests include small buffers and simultaneous large traffic. Replacing all
unbounded queues with blocking bounded sends would risk recreating that bug.

The useful transport change is nonblocking, byte-based admission with reserved
capacity for cancel/heartbeat/receipts, plus fair scheduling of independently
identified sessions. Any chunked bulk protocol needs reader-first capability
rollout and bounded reassembly. A single already-started WebSocket data frame
cannot be preempted merely by prioritizing the next application message. Retain
sequencing within each operation, authentication and binding fences. Measure
queued bytes/age, frame bytes/type, writer duration, remote operation duration
and receipt duration before choosing chunk sizes or flush batches.

Both writer implementations currently call `send` for each frame; bounded
micro-batching of adjacent bulk frames might reduce flush/syscall overhead. This
is a secondary candidate requiring a benchmark: interactive messages must not
wait for a batch timer, and batching does not fix excess file bytes or serial
RPCs. Do not enable generic compression based only on base64 size; measure CPU,
compression ratio and interactive latency, and exclude sensitive data contexts.

The previously recorded mobile persist time was 56–87 ms while confirmation took
30–33 s. This proves delayed confirmation, not its exact network cause. Changing
confirmation timeouts or restarting workers is not a throughput fix. Trace one
request through durable acceptance, dispatch, Provider acceptance, first token
and final receipt. Keep log dimensions bounded and do not log prompts, file
contents, environment values, descriptors or authentication.

Keep detailed stage traces bounded on the hosts and export aggregate counters
and latency histograms, rather than a Cloudflare/D1 write for every tool RPC.
Use a short, explicit retention policy for sampled failure traces. Request IDs
belong in sampled diagnostic traces, not high-cardinality metric labels. This
adds timing attribution without duplicating Stormbird's route-policy engine.

## Practical delivery and acceptance

Use fixed scenarios: short read; two ranges of a generated 1 MiB file; search
pagination; safe small conditional edit with a concurrent-writer conflict; quiet
60-second command; streaming command; cancel while bulk output is queued; and
reconnect after acceptance with the same operation identity. Run Codex and
Claude independently, with runtime OVH and each supported target. Report request
count, wire bytes, queue age, first output, total task time and resource use.
Keep failure and negative-security cases in the comparison.

For transport changes, compare idle interactive latency with large generated
output from a different session. Require no lost cancel, no duplicate execution,
no cross-binding access, bounded memory and no heartbeat starvation. A latency
budget must be frozen before implementation; this research has not changed
existing production acceptance budgets.

Deploy the narrow owning component: Provider adapter changes need a new signed
Plugin; host verification changes need a Machine component; transport changes
may need both Controller and Machine. Admit new sessions on the new generation
while retaining active workers. Do not pretend an installation update migrates
an already-running session. Revert by an owning-component predecessor receipt or
a signed successor repair; keep permanent enrollment and workspace identities.

Already effective SSH connection reuse should stay in place. Current healthy
managed-alias medians are approximately 0.35–0.37 s across the three targets; an
established stream is approximately 0.16–0.17 s. Those are short SSH samples,
not native RPC timings. Native transport should first remove unnecessary RPCs
and payloads rather than paying more SSH command startup. Physical phone network
acceptance, daily network tail latency and full real-task optimization speedups
are not established by this report.
