# Claude native tools through Mods

Claude Code Plugin 3.3.0 replaces the execution MCP server and tool aliases with
the native `tool.call` Mod interface in the pinned Claude Code 2.1.287. Native
authentication, history and inference remain on the Agent Machine; Cowboy's
existing execution binding owns files and processes on the target Machine.
The native executable, ACP 0.84.0, Agent SDK 0.3.284, Provider runtime 1.1.11 and
Plugin component-release pin 3.35.0 remain unchanged.

```text
Claude native Bash / Read / Edit / Write / Glob / Grep / NotebookEdit / TaskStop
  -> private tool.call Mod
  -> per-process authenticated Unix socket
  -> existing Cowboy worker execution connection
  -> target Machine keeper and executor
```

The model receives native tool names and schemas, with concise `tool.describe`
descriptions. There is no execution MCP server, alias table or additional MCP
tool schema. This preserves native read-only scheduling. The existing facade
still orders operations on the same file and keeps persisted read stamps and
process output cursors. Uncertain mutations are never resubmitted automatically.
File and notebook results match the exact native output schemas; bounded diffs
are computed from the bytes involved in the accepted write.

## Native constraints discovered in acceptance

- Mods HTTP has a fixed 30-second timeout, including Unix sockets. A call waits
  at most 20 seconds per HTTP response; an unfinished response returns its
  already-admitted identity. The same Mod invocation then observes `/result`.
  This is internal waiting, not another model turn or another remote submission.
  Identities, active calls and retained result bytes are bounded. Completed
  results are consumed once and abandoned results expire; their spent identity
  remains, so an ambiguous outcome cannot replay a mutation.
- A Mod exception, timeout or invalid native result normally skips the handler.
  The tool hook has an immediate `.catch` denial. Actual malformed-result and
  broken-bridge fixtures verify that the native tool body does not run. Native
  initialization is withheld until the context Mod and its private socket have
  passed readiness without a model request. Bare mode, replacement hooks/tools,
  external MCP servers and execution-changing controls remain refused.
- Compaction directly rereads native Read/Edit paths outside `tool.call`.
  Disabling ordinary attachments does not cover this path. A `prompt.attachment`
  hook suppresses the native `file` snapshot on every rendering, including cold
  resume. The real summary remains; a subsequent explicit Read obtains target
  content. This is tool/context binding, **not process-wide filesystem isolation**.
  The native process still owns its local auth/history and may internally read
  a runtime file; that file snapshot is not authoritative project context.
- `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` also blocks Mods socket HTTP.
  Only the bound native child's environment drops that aggregate flag; explicit
  telemetry, error reporting and updater switches remain disabled. Native title
  generation can issue its usual separate model request. The fixture accounts
  for it separately and checks it for runtime-context leakage.
- This native release has no TaskOutput tool. Running Bash returns a task ID
  and `cowboy-task://` output handle. Read observes that handle and TaskStop
  cancels it. The handle survives cold resume; it is not an OVH file path.

## Verification and release

The package gate drives the actual bundled ACP/native CLI through the Cowboy
worker, two disposable Machines and retained target keeper. A scripted loopback
model supplies tool calls; it uses no subscription credentials or paid inference.
It covers target bytes, quotes/Unicode/CRLF, conflicts, notebooks, images,
large reads, concurrent FIFO searches, task output and cancellation, 35-second
transport loss, hot configuration, native resume/compaction and refusal paths.
Final release receipts additionally bind the packaged bytes, old/new worker
coexistence, current/recovery/cold Catalog readers and the exact OVH installation.

The full build also records app-shell 1.1.18 in component matrix 3.36.0 for the
already-committed update countdown and thin download changes. This repairs their
missing source-digest record without changing Web behavior or any Plugin's
component dependency. Historical matrix entries are untouched.

Existing sessions retain their generation. A new remote Claude session uses the
installed Plugin; local sessions retain the upstream ACP launch path. No core
component restart is needed. Native subagents, project hooks/skills, implicit
file attachments, plan files and PDF extraction remain outside this execution
lane. Whole-file transport and non-atomic conflicts with unrelated writers remain.
No model token/turn savings, cross-host latency, physical-device or macOS native
execution acceptance is inferred from these tests.

The integration uses the upstream [Mods events](https://code.claude.com/docs/en/plugins/mods/events),
[host API](https://code.claude.com/docs/en/plugins/mods/api) and generated types
from the exact private native executable, rather than patching that executable.
