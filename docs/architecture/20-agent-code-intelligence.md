# Agent Code Intelligence from the Code Review Engine

**Status: design proposal (2026-11-05), not implemented.** Agents become consumers of the
[Code Review Engine](19-code-review-engine.md) capabilities. "Language engine"
means the Engine's LSP-backed implementation, which is the
[isolated Zed component](14-zed-code-provider.md) today; agents never see it.

## Problem

The language engine keeps one warm, trust-gated project per Machine worktree:
language servers, per-buffer diagnostics, hover, definition / declaration /
type definition / implementation / references and document outline already
answer the Web Code pane (`/api/code/sessions/{id}/intelligence/*`,
`plugins/zed/adapter`). Agents cannot use any of it:

1. **Agents are blind to the warm backend.** Claude, Codex, DeepSeek Harness
   and Grok sessions re-run compilers or grep to learn what the language engine
   already knows. The cowboy MCP read SDK has no `code` namespace.
2. **The provider learns about agent edits late.** It observes only
   filesystem deltas, although Cowboy itself executes every remote write:
   Claude's native Read/Edit/Write go through the private `tool.call` Mod and
   facade (docs/experiments/claude-mods-execution-2026-10-02.md), and Codex's
   native `apply_patch` and commands are dispatched to the target Machine
   (`native_apply_patch_and_command_use_target`). ACP `tool_call` updates also
   carry `locations` and `diff` content before the filesystem event lands.
3. **Diagnostics are per open buffer only.** There is no project-wide summary,
   no notion of "new since this agent's edit", and `diagnosticsState` can be
   `unobserved` (cold or restricted) without that reaching any agent.

## External evidence (late 2026)

- Claude Code ships a native `LSP` tool and post-edit "Found N new diagnostic
  issues" injection when a plugin declares `lspServers` (stdio `command`,
  `extensionToLanguage`, `diagnostics: true`). Mods (`tool.call`,
  `tool.describe`, `session.append`, `$.tool.register`, `prompt.attachment`)
  can observe, rewrite or answer native tool calls.
  <https://code.claude.com/docs/en/plugins-reference.md>,
  <https://code.claude.com/docs/en/plugins/mods/reference>
- Codex has no native LSP (openai/codex#8745: internal experiments "hadn't
  shown the benefits they had expected"). Its hooks support
  PreToolUse/PostToolUse on `apply_patch`, `mcp_tool` hooks and
  `additionalContext` (about 2,500 tokens; `async` hooks deliver at the next
  safe point). <https://developers.openai.com/codex/hooks.md>
- OpenCode turned built-in LSP off by default: servers drift out of sync,
  use memory and slow agents. <https://opencode.ai/docs/lsp/>
- Serena v2 replaced one-MCP-tool-per-operation with a single REPL facade
  (`s.lsp.*`) for composition and context economy, which matches Cowboy's
  CodeAct read programs. <https://github.com/oraios/serena>
- arXiv 2608.13568 (preliminary): agents chose LSP navigation for only 0-6%
  of localization steps; LSP helped reference F1 only where grep is noisy;
  returning **source lines inline** with each location recovered edit success
  and cut tokens 19%; a cold pyright index silently returned incomplete
  references. Cross-project consensus favors **pushed, delta-only, bounded
  post-edit diagnostics** over many navigation tools.
- Zed's own agent addresses symbols as `{file_path, line, symbol_name}`,
  exposes a project-wide diagnostics summary, and tells the model to stop
  after one or two attempts at one diagnostic.
- Raw LSP multiplexing across clients (lspmux) drops server-to-client
  requests and makes per-client `didOpen` unsound; avoid it.

## Principles

1. **One warm backend, many consumers.** The language engine remains the only
   language-server owner per worktree. Agents never start their own LSP for a
   Cowboy worktree and never see which implementation answers.
2. **Disk stays the single write channel.** Agents keep writing through their
   native tools to the target. Language-engine buffers held by Cowboy remain read-only
   leases; the private native buffer-sync primitive stays a Code-pane effect,
   never an agent write path. This keeps the existing reload/ownership
   guarantees in [native buffer sync](../plugin-native-buffer-sync.md) intact.
3. **Cowboy's write path is the change signal.** The executor that applied a
   write (Claude facade, Codex target dispatch) or the ACP tool call
   `locations`/`diff` names the exact paths. That signal asks the provider to
   refresh those paths immediately instead of waiting for watcher debounce.
4. **Deltas, not dumps.** Agents receive diagnostics introduced by their own
   edits since a per-session baseline, bounded and with source lines.
   Pre-existing problems are summarized as counts.
5. **Honest freshness.** Every answer carries `state`:
   `fresh | settling | warming | restricted | unavailable`. Empty results are
   never reported as clean unless the provider observed the exact file version.
6. **Provider-neutral core, native adapters at the edge.** One Controller API
   serves every agent; Claude and Codex add thin native delivery adapters.

## Architecture

```text
 agent native tool ──► Cowboy executor (target Machine) ──► disk write
        │                         │
        │ ACP tool_call            └─► CodeChange{session, paths, write_id}
        ▼                                     │
   Controller ─────────────────────────────── ▼
   code intelligence service ◄──────── Machine language engine (warm LSP)
     • baseline per (session, path)            • read-only buffer leases
     • diagnostics delta / summary             • version-matched diagnostics
     • navigation with inline lines            • outline, hover, workspace symbols
        │
        ├─ L1 pull: cowboy MCP  cowboy.code.*       (every agent)
        ├─ L2 push: post-edit delta                 (Claude Mod, Codex hook)
        └─ L3 surfaces: transcript badge, Code pane follow, turn summary
```

### Change signal

The executor that applies a write on the target emits a Machine-local
`CodeChange{session_id, worktree, paths, write_id}` to the co-located language
engine after the bytes are durable. ACP `locations`/`diff` from providers
whose writes Cowboy does not execute (local sessions, DeepSeek Harness) feed
the same message from the worker. The adapter refreshes or opens a
read-only lease for each path, so the next diagnostics answer is
version-matched to the bytes the agent wrote. The filesystem watcher remains
the reconciliation path; the signal only removes latency.

### Baseline and delta

The service records the diagnostics of a path the first time the session reads
or edits it in a turn (lazy baseline; a turn-start full snapshot is too
expensive). `delta(session, paths)` returns diagnostics present now and absent
from the baseline, matched by `(source, code, message, symbol-relative range)`
so moved lines do not count as new. Baselines are bounded per session and
expire with the session's worktree lease.

### Settle semantics

`delta` waits until every requested path reports diagnostics for the written
version, using pull diagnostics (`textDocument/diagnostic`) where the server
supports it and published diagnostics otherwise, up to a default 1.5 s
(maximum 5 s). It then returns `settling` with whatever is known rather than
blocking the agent. `warming` and `restricted` are reported explicitly.

### Output budget

A delta lists at most 10 new errors and 5 new warnings, each with
`path:line`, message, source/code and the source line. Everything else is
counts. A push adapter caps its injected text at 2,000 characters. The same
unresolved diagnostic is repeated at most twice per session before it is
reduced to a count, mirroring Zed's "one or two attempts" guidance.

## Delivery

### L1 - pull, every agent: `cowboy.code.*`

A read-only namespace in the existing cowboy MCP CodeAct SDK, scoped to the
caller's own session worktree (no other session's worktree and no path
outside it):

| Operation | Returns |
| --- | --- |
| `code.diagnostics({paths?, since?: "baseline"|"none"})` | per-path state, new/all problems with source lines, counts |
| `code.summary()` | project-wide error/warning counts by file, provider state |
| `code.definition({path, line, symbol})`, `code.references(...)`, `code.implementations(...)` | locations with the source line inline |
| `code.outline({path})`, `code.symbols({query})` | symbol lists |
| `code.hover({path, line, symbol})` | signature/type text |

Symbols use a `{path, line, symbol}` locator, as Zed's own agent does, rather than columns. Programs
compose and filter results in one call, so the agent pays for schemas once
(the read SDK is already loaded) and only for projected results.

### L2 - push after edits

- **Claude (Mod).** The Cowboy `tool.call` Mod already answers Edit/Write/
  NotebookEdit through the facade. After the write it calls
  `delta(session, [file_path])` and appends the bounded text to the native
  result through `session.append` (tool_result content rewrite). The native
  result shape is unchanged. Whether `session.append` or a native diagnostics
  attachment (`prompt.attachment`) renders better is decided by experiment E2.
- **Codex (hook).** Codex sessions run with a provider-owned `CODEX_HOME`. Its
  managed hooks add a PostToolUse hook on `apply_patch` that calls the
  `cowboy` MCP (`mcp_tool` hook) and returns `additionalContext`. It never
  blocks (`decision` absent); `async` delivery is used if measured latency
  exceeds the settle budget.
- **Other agents.** Pull only, plus the turn summary below.

### L3 - surfaces

- Each edit tool call in the transcript gets a badge (`+2 errors`, `clean`,
  `settling`) from the same delta, for every provider.
- At turn end Cowboy records one per-turn delta. The Code pane can follow the
  agent's most recent edit location.
- No automatic Stop-hook gate in the first phases: blocking a turn on
  diagnostics risks loops on monorepo false positives.

## Rejected or deferred options

- **Native Claude `lspServers` shim** (`command: cowboy lsp-bridge`): Claude
  would get the native `LSP` tool and diagnostics for free, but Cowboy would
  have to implement an LSP server facade, Claude's own `didOpen/didChange`
  would compete with the read-only lease model, and it is Claude-only.
  Deferred to experiment E3; adopt only if it beats L1+L2 on measured cost.
- **Agents writing through Zed buffers**: lower latency and exact attribution,
  but needs the unfinished synchronization authority and breaks the
  single-writer rule.
- **LSP multiplexing** of one server to many agent-owned clients: unsound.
- **Speculative `checkEdit(patch)`** in a scratch buffer: highest fidelity,
  most RPC and LSP churn. Revisit after L2 data.

## Security and isolation

- The GPL process boundary of the Zed component is unchanged; new operations are versioned
  messages on the existing private socket.
- Scope is the caller session's worktree on its execution Machine. Restricted
  (untrusted) worktrees return `restricted`; no project language server runs
  because an agent asked.
- Diagnostic text is worktree-derived content and is treated like file
  content: it never crosses into another session or into Controller logs.
- Hooks and Mod additions are Cowboy-owned provider configuration; project
  hooks remain governed by the existing project-hook rules.

## Phasing and measurement

| Phase | Scope | Exit evidence |
| --- | --- | --- |
| P0 | Adapter: project diagnostics summary, workspace symbols, version-matched per-path diagnostics; Controller `cowboy.code.*` read SDK | Conformance on the pinned Zed pair; cold/warming/restricted states visible |
| P1 | Executor `CodeChange` signal, baselines, `delta`; transcript badges | p50/p95 edit-to-fresh latency; false-new rate on moved lines |
| P2 | Claude Mod and Codex hook push | A/B on recorded tasks: fix-turns per broken edit, tokens per task, compile-and-rerun commands avoided |
| E3 | Claude `lspServers` shim experiment | Must beat P2 on tokens without lease conflicts |

Telemetry counts calls, states and latencies only, never diagnostic text.

## Ownership and deletion triggers

The language engine, Machine executor and Controller own the service; provider
Plugins own only their delivery adapters. Delete the Codex hook adapter if
Codex ships native post-edit diagnostics that can target a remote LSP; delete
the Claude Mod push if Claude Code can consume a remote language server
natively with equal freshness guarantees.

## Open questions

1. Which executor event is the earliest point where bytes are durable for both
   the Claude facade and Codex target dispatch, and can both reach the
   co-located adapter without a Controller round trip?
2. Should baselines be per session or per worktree when a review call and its
   parent share one worktree snapshot?
3. Is `session.append` tool_result rewriting stable across Claude cold resume
   and compaction, or does it require the attachment path?
