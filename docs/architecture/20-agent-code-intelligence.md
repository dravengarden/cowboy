# Agent Code Intelligence from the Code Review Engine

**Status: partially implemented; remaining gates below (2026-11-06).** r3 folds in two
static design reviews (Fable 5.1, then Codex Astra Max). Agents become
consumers of the [Code Review Engine](19-code-review-engine.md) capabilities.
"Language engine" means the Engine's LSP-backed implementation, the
[isolated Zed component](14-zed-code-provider.md) today; agents never see it.

## Problem

1. **Agents are blind to warm code intelligence.** Claude, Codex, DeepSeek
   Harness and Grok re-run compilers or grep for what Review already knows; the
   cowboy MCP read SDK has no `code` namespace.
2. **Review learns about agent edits late.** It polls; there is no core
   worktree watcher (`notify` is used only for auth and Catalog watching).
3. **Diagnostics are per open buffer, last observed.** `diagnosticsState` is
   `unobserved | observed`; a refresh trigger's acknowledgement proves no
   completed analysis (`docs/plugin-owned-buffer-reads.md`).

## External evidence (late 2026)

- Claude Code: native `LSP` tool via plugin `lspServers`; Mods can observe,
  answer and append around native tools.
- Codex: no native LSP (openai/codex#8745); PreToolUse/PostToolUse hooks with
  `additionalContext`.
- OpenCode disabled built-in LSP by default (drift, memory, slowness).
- arXiv 2608.13568: agents rarely choose LSP navigation; inline source lines
  matter; cold indexes silently return incomplete references.
- Consensus: pushed, delta-only, bounded post-edit diagnostics are the
  highest-value signal; avoid LSP multiplexing.

## Principles

1. **One warm backend, many consumers.** Agents never start their own LSP for a
   Cowboy worktree and never see which implementation answers.
2. **Disk is the only write channel.** Agents write through their native tools;
   Cowboy-held language buffers are read-only; no LSP multiplexing.
3. **Honest answers.** Every answer carries `state`, `precision` and coverage.
   Empty is never "clean" and never "no references".
4. **Separate authorities.** Observing, acquiring native resources,
   synchronizing them and delivering to a model are four distinct grants.
5. **Analysis on the Cowboy side, rendering at the edge.** Native adapters
   (Claude Mod, Codex hook) only render an already available bounded result.
6. **Code Review failure never touches the turn.**

## Architecture

```text
 agent native tool ─► Cowboy executor (execution Machine) ─► disk
                              │ receipt                 ▲ watcher
                              ▼                         │
                 Code Observer (execution Machine) ─────┘
                 • observation stream, baselines, analysis cache
                 • asks the co-located implementations
                              │ standing observation continuation
                 Controller: policy, trust, authority, projection
        ┌─────────────────────┼───────────────────────────┐
   L1 pull: cowboy.code.*   L2 push: Mod/hook render   L3 badges, Code pane
```

### Authority

- **Observation continuation.** The Controller issues it per (session
  incarnation, complete execution binding, worker grant, Machine connection,
  trust scope) and installs it on the execution Machine beside the call
  gateway grant (`docs/managed-agent-calls.md`, "Grant"). The Observer acts
  locally on receipts and watcher events only under a current continuation;
  replacement, revocation, retarget, worker replacement or deletion drops its
  baselines and pending deliveries. Disconnection suspends delivery; it does
  not authorize replay.
- **Pull authority.** Each `cowboy.code.*` read is admitted by the Controller
  through the gateway, re-deriving authority per call; the continuation alone
  never authorizes a model-visible result.
- **Acquisition authority** (opening native buffers on an agent's behalf) is a
  separately admitted, bounded capability. Today owned-buffer open requires a
  current product Operator (`components/server/src/server/code_buffers.rs`,
  `check_open`); a worker read continuation is not that grant and must not be
  substituted for it.
- **Synchronization authority** remains the Service confirmation contract
  (`docs/plugin-service-buffer-sync.md`); automatic refresh never borrows it.
- **Scope.** Agents read only their own session's worktree root through a
  **non-projecting resolver**: the Review resolver projects
  `projects/<name>/...` onto registered checkouts outside the worktree
  (`src/code_review.rs`, `language_buffer_key`). The actual root is verified
  before acquisition and before every source read; the same output-scope check
  applies to summaries, symbols and diagnostic related locations. External
  URIs, escaping paths and symlinks fail closed.
- ACP-provided identifiers never establish authority.

### Change observation

- The Machine **watcher** (new core component) covers sessions with agent
  intelligence or an open Code pane. It has an observer incarnation, a
  retention low-water mark, explicit `gap`/`reset` responses and a
  scan-plus-event reconciliation barrier. During a gap, freshness and
  attribution of dependent comparisons are invalidated, including unchanged
  open files. Scan exclusions (13-code-review "Scan policy") are not
  invalidation exclusions: ignored generated inputs still affect analysis.
- **Executor receipts** are latency accelerators: binding incarnation, turn,
  tool identity, monotonic sequence, operation kind and before/after content
  identities, emitted after durability. ACP `locations`/`diff` are unverified
  hints only. Intermediate versions are superseded; delete/recreate is a new
  identity.

### Freshness and coverage

- `fresh` requires causal evidence, per participating server, that it analyzed
  the exact content **and** an analysis input frontier covering relevant
  dependency/configuration changes. Otherwise the answer is
  `document-current, project freshness unknown`, `last_observed` or `stale`.
  Quiet periods, timeouts and content equality never produce `fresh`.
- The participant set is fixed before analysis; a failed server cannot leave
  the denominator.
- Summaries state their observed scope (usually open files); they are never
  project totals unless a `check@1` result covers the project.
- Save-triggered analyzers (rust-analyzer flycheck, gopls vet) are reported per
  server as `stale` when their analyzed version is older than the content.
  Automatic refresh never counts as a save.
- **Analysis-frontier evidence (implemented, private Zed server 1.7.0).** For
  every diagnostics set a server publishes into a buffer, the server records
  the version Zed mapped it onto: the exact analyzed snapshot when the LSP named
  a version, otherwise the text at publication (an upper bound), plus saved
  version, disk-based and carried-over flags (`CowboyAnalysisFrontier`). The
  adapter lists every server running for the buffer and compares each against
  the mirror its diagnostics are read from and the stamp of the set it holds:
  `document_current` only for a versioned, not disk-based, not merged set of
  exactly that text; `stale` when the analyzed version is older; otherwise
  `unknown` with a reason (`unpublished`, `in_flight`, `unversioned`,
  `save_triggered`, `merged`). Pulled diagnostics (`textDocument/diagnostic`,
  for servers that advertise that capability) carry the document version the server
  last received when the pull was sent (private server 1.10.0); the server
  answers after every earlier change notification. `code.diagnostics` reports a file as
  `document_current` only when every listed server is. This is document
  evidence only; the input frontier for dependencies and configuration is
  still open, so nothing reports `fresh`.

  Hawk's gopls v0.22.0 does not advertise diagnostic pull. A direct stdio
  probe on 2026-11-06 showed that its pushed diagnostics omit the version for
  `didOpen` version 0, but include version 1. Private server 1.11.0 starts
  document versions at 1 and only treats a publication as versioned when the
  exact LSP snapshot is retained. Zed's special version-0 fallback when no
  snapshot exists is not analysis evidence. Hawk's production MCP route returned
  `document_current` for gopls after the 1.26.0 installation and Controller
  activation; see the [exact release evidence](../releases/zed-initial-version-2026-11-06.md).
  This remains document evidence, not project freshness.

### Acquisition: gated

The adapter has a process-wide admission fence: an unresolved native Open,
navigation or synchronization refuses every new open on that Machine, with no
recovery path yet (`docs/plugin-native-open-once.md`,
`docs/plugin-owned-navigation.md`, `docs/plugin-buffer-sync-owners.md`), and
reopening a path can adopt a shared native buffer. Automatic paths would
multiply Unknown outcomes. Therefore **no automatic acquisition** (pull, badge
or push) until all of:

1. fences scoped to the native-ID alias set of the target, never to pathname
   equality (open admission done in Zed 1.24.0; unresolved navigation stays
   process-wide);
2. independently authorized Unknown recovery and cleanup tied to exact native
   process identity, surviving Controller restart and worker replacement
   (open recovery implemented in Zed 1.23.0: an automatic query of the same
   server process, see `docs/plugin-native-open-once.md`; navigation and
   synchronization Unknowns remain);
3. proven isolation from and priority below interactive Code-pane acquisition;
4. the per-Machine process budgets of [19](19-code-review-engine.md).

Until then agents receive only observations that already exist safely
(buffers a Code pane holds, structure tags, `check@1` results), or
`unavailable`.

**Agent acquisition (implemented, Controller with Zed 1.24.0).** The owner
chose the session's "Project language servers" switch as the acquisition
authority (2026-11-06). `code.diagnostics` opens a file of the session's own
worktree that no Code pane holds under an acquisition-specific legacy owner
(`cowboy-agent-<session-sha256>-<nonce>`). Retries retain that identity; a new
holding or Controller restart uses a new identity so a delayed old Close cannot
release the replacement. The Machine groups nonce-bearing owners by session
for limits. The internal request explicitly declares `ownerProtocol: 2`;
legacy requests retain Controller-only cleanup for rollback compatibility.
Older Machines reject the additional field before native effects. Agent
holdings are a bounded share below interactive use:
4 per session, 8 per Machine, at most 2 agent-warmed worktrees per Machine;
an independent Controller task checks every 30 seconds for holdings idle for
5 minutes or with the switch off. Cleanup waits for in-flight opens, rechecks
idle state before closing, and frees capacity only after a confirmed close.
An unknown close keeps the owner fenced and counted; it is not retried under
the same owner or redirected to a replacement worktree. Reads do not wait for
unrelated idle cleanup. Gates 1 and
2 hold for opens (target-confined admission, same-process recovery). The new
Machine admission path additionally measures native process count, RSS and CPU
before automatic warming, as specified in [19](19-code-review-engine.md#resource-budgets).
It requires the containing Machine implementation; the earlier installed path
had only Controller counts. These are admission thresholds, not hard resource
limits. An exited adapter can be replaced for an explicit new open, even when
old leases retain its original incarnation. Those leases are never retargeted;
the old process group remains measurable until its last owner releases it.
A stop request is not evidence of zero descendants. Navigation and
synchronization Unknowns keep their own fences.

Hawk activated this admission and runtime replacement implementation from
`35d9effa0fd9477a58e05e43bbfc75cc7f99a57c` on 2026-11-06. Controller transaction
`1793969604518101125-35d9effa0fd9` and Machine transaction
`1793969644803941790-35d9effa0fd9` both reported `succeeded` / `committed`;
the Machine retained `worker-4f248c87703274b63cd9`. A production agent read of
`machines/cmd/cowboy-release-activate/main.go` returned `acquired: true` and
gopls `document_current` after activation. This accepts successful admission,
not saturation behavior: the latter is covered by delegated-cgroup tests of
RSS/process/CPU refusal, interactive access and original/replacement accounting.

The Machine also records automatic holdings before ensure/open dispatch and
enforces the 4/8/2 count limits independently of Controller memory. Its own
30-second maintenance loop releases confirmed Opens after five idle minutes,
using the exact original runtime and original worktree spelling. Thus a
Controller restart does not reset Machine admission counts or leave confirmed
idle Opens dependent on another agent read. Interactive owners are preserved.
Uncertain Opens and Closes remain counted; maintenance does not retry Close or
infer completion from a missing owner. A release in progress prevents reuse of
that owner. Independently authorized recovery of those uncertain holdings
remains separate work. This is Machine-process state, not persistence across
a Machine restart.

Hawk activated the Machine-owned idle cleanup and retained process retirement
from `2b154933ef0313f2590acf4a3ce0ab41fd1d68ea` on 2026-11-06. Controller
transaction `1793977879629449763-2b154933ef03` and Machine transaction
`1793977908771016764-2b154933ef03` both reported `succeeded` / `committed`,
retaining `worker-4f248c87703274b63cd9`. All four Machines reconnected;
health returned `ok` and the public SPA returned HTTP 200 with `no-store`.
A subsequent production read of `machines/cmd/cowboy-release-activate/main.go`
returned `acquired: true` and gopls `document_current`; the actual adapter
and server processes used Zed 1.28.0 generation
`dad653c9635f6fb850e22dea62840ccb9b1a55dbc24453cc25215d17b71a36b7`.
After the idle interval, the passive summary returned `tracked: 0`,
`counts: null`, and both observed adapter/server PIDs had exited. This checks
idle release without another acquisition; original-cgroup removal and detached
descendant accounting are established separately by the delegated-cgroup tests.
This does not establish Unknown recovery or worker-generation acceptance.

The separate worker candidate from that same revision subsequently passed
Codex native execution (34 checks), Claude native execution (96), managed
read-only Claude (7), session/recovery (39), logs (11), connected Code (38),
and both Providers' distinct-generation coexistence and descendant drain.
The [immutable acceptance record](../experiments/code-worker-pin-2026-11-06.json)
binds `worker-39e0360cdcbff26cfa61` and the supplied release artifacts. These
isolated fixtures use no production credentials and do not establish a
production pin activation. The session fixture uses a short `/tmp` root;
the initial long Nix-shell temporary path failed Unix socket startup before
the scenario ran and is not included as accepted evidence.

Hawk subsequently activated pin commit
`53fcbb9500cc8648298ec221dbdd9f810a800615`, transaction
`1793980312127758480-53fcbb9500cc` (`succeeded` / `committed`). Both existing
idle workers actually cut over from `worker-4f248c87703274b63cd9` to
`worker-39e0360cdcbff26cfa61`, verified by their executable paths, generation
arguments and Machine cutover logs. Six missing-file production reads left
`tracked: 0`; four subsequent existing Go files each returned `acquired: true`
and `document_current`. The Machine reconnected ready, `/healthz` returned
`ok`, and the public SPA returned HTTP 200 with `no-store`. This accepts the
Hawk worker cutover, not an upgrade of the current OVH-hosted Provider session.

Automatic acquisition now preflights regular files before selecting a runtime
or reserving an owner. Missing files, directories and symlink components return
a definite pre-dispatch refusal, avoiding an orphan reservation for inputs
already known to be unusable. Native resolution remains authoritative: a file
can change after preflight, and an uncertain dispatched request still retains
its original ownership and capacity.

Passive language/hover/navigation/symbol requests also refuse a cold installed
runtime with `not open`. The first diagnostic observation therefore cannot
launch a backend before the automatic acquisition budget. Reads through an
already configured legacy socket retain their existing behavior; this does not
launch a process or confer new buffer ownership.

Hawk activated the cold-read guard from
`a408035c6cb1fbf174d006c1ad1efab900f091f4`, transaction
`1793981545889231556-a408035c6cb1` (`succeeded` / `committed`). Starting with
no installed Zed processes, two missing-file production observations returned
`not_open`, kept `tracked: 0`, and started no Zed process. A subsequent real
Go file acquired successfully and returned gopls `document_current`. The
Controller and both existing worker PIDs remained unchanged, the Machine
reconnected ready, and health and public SPA checks passed.

Machine maintenance observes retained synchronization and navigation operations
through their original runtime and native reference, independently of whether
the initiating control connection remains alive. Each 30-second pass selects
at most four operations per registry in round-robin order, with a two-second
query deadline per operation. Busy operations and routes are skipped. The
maintenance path sends only Query; it never repeats Apply, Execute, Retire or
Release, selects a replacement runtime, or delivers a result to a connection.
Public queries still require the original owner. Valid terminal evidence uses
the same validation as foreground queries and can clear the original sync fence
or confirm a previously dispatched retirement/release. Missing evidence retains
the original ownership and capacity. Terminal results are not automatically
retired, and this does not recover state lost with the Machine process or solve
unknown legacy Opens and Closes.

Hawk activated this maintenance observation from
`bc6262f79936e1b75d7cc0ccc3d6af8a6afb9c60` on 2026-11-07, transaction
`1794018627362793557-bc6262f79936` (`succeeded` / `committed`). The complete
pinned gate passed. The host release retained `worker-39e0360cdcbff26cfa61`;
the Controller PID and both existing workers' PIDs, executables and arguments
remained unchanged. All four Machines reported ready, health returned `ok`, and the public
SPA returned HTTP 200 with `no-store`. Lost-reply recovery, original-owner
refusal, persistent uncertainty and fair bounded selection are fixture evidence;
this activation did not inject failures into production sessions.

### Baselines, attribution, matching

- Baselines are immutable, content-matched captures at a read or pre-write
  boundary (the facade and executor see pre-write content). Without one the
  comparison is `unknown`.
- **Hunk overlap** orders diagnostics within the budget (`in_edit` before
  `elsewhere`); it never asserts that an edit caused a diagnostic and never
  replaces the baseline comparison.
- Identity (source, code, anchor) is separate from mutable attributes
  (severity, message); severity escalation is a change. Matching is
  multiplicity-aware with versioned anchor mapping; without `code`, provenance
  and ambiguity are preserved rather than hashing messages.
- Dependent files: a bounded worktree diagnostic sequence plus project epoch,
  classified `attributable | newly_observed | concurrent`; an empty delta means
  "nothing new observed there".

## Delivery

### L1 pull: `cowboy.code.*`

A read-only namespace answered by **direct gateway reads**, not by waiting
inside CodeAct programs (two concurrent programs per host, 20 s;
`docs/agent-api.md`, "Contained runtime"). Programs may compose these reads but
never wait for analysis: a read returns the current observation with its state
immediately.

| Operation | Returns |
| --- | --- |
| `code.search(query, {regex?, case_sensitive?, path?, limit?})` | bounded content matches with lines (core; implemented) |
| `code.outline(path)`, `code.symbols(query, {limit?})` | syntax structure (tree-sitter tags; implemented, `heuristic`); language-engine refinement later |
| `code.diagnostics(paths)` | implemented: diagnostics and counts with a per-file state from per-server analysis-frontier evidence (`document_current`, `stale`, otherwise `last_observed`), or `not_open`/`restricted`/`refused`/`unavailable`. A file no Code pane holds is acquired for the agent (`acquired: true`, see Acquisition) and waited on for up to 4 s. Baseline comparison comes with P2 |
| `code.summary()` | implemented: passive counts over this session's tracked files (`coverage: agent_observed_files`), without acquisition or idle-time refresh. No usable observations means `counts: null`/`unobserved`; `partial` counts exclude unavailable files. Counts are never project totals, and Controller restart discards the tracked set until files are read again |
| `code.definition/references/implementations(locator)`, `code.hover(locator)` | after owned-navigation acceptance only |

Locators are an exact UTF-16 position or an opaque versioned symbol handle
bound to a content snapshot; `{path, line, symbol}` is accepted only when
uniquely resolvable, otherwise candidates or `stale/ambiguous`.

### L2 push after edits (opt-in per session)

- **Claude.** Captured native research: while a Mod `$.http.fetch` is pending
  the native runtime processes no other work, including parent turns and
  TaskStop (`docs/experiments/claude-native-agent-lifecycle-2026-10-06.json`).
  So the Mod returns the canonical write result unchanged, never awaits
  analysis, and later consumes an already computed bounded result in short
  (≤1 s) exchanges, delivered as its own row through the existing async-note
  path (`plugins/claude-code/runtime/context-mod.js`, `deliverAsync`), never by
  rewriting a tool result.
- **Codex.** Existing research proves app-server-owned command hooks only, not
  project-hook exclusion (`docs/experiments/codex-native-hooks-research-2026-10-05.json`).
  A hook is accepted only if conformance proves exact Cowboy-owned hook hashes
  are trusted while project hooks are refused under the session's
  `trust_level: "trusted"`, and that it reaches the original execution binding
  without credentials in the runtime. Otherwise Codex push is delivered from the
  Cowboy side at the next safe point.
- **Coverage.** Push triggers on any tool completion with a watcher-observed
  change set since the tool started (shell, formatters), not on
  `apply_patch`/Edit only.
- **Delivery identity.** Recipient (main agent; native subagents excluded
  initially), conversation/context identity, idempotent delivery ID and states
  `queued | appended | consumed | unknown`. Only evidenced model deliveries
  count against repetition limits. `context_cleared` drops pending deliveries;
  compaction and cold resume keep them as labelled historical observations.
- **Payload.** Bounded (≤10 errors, ≤5 warnings, source lines, ≤2,000 chars),
  structured, with every repository-controlled field (messages, source lines,
  paths) escaped as untrusted data inside a fixed presentation. Injection
  fixtures cover reminder delimiters and control characters.
- Late results are delivered at the next safe point within bounds or the turn
  is marked "validation pending"; turns are never reopened; superseded results
  never attach to later edits.

### L3 surfaces

Per-edit transcript badges (`+2 errors`, `no new observed`, `settling`,
`unknown`) for every provider; per-turn record. Following the latest edit in
the Code pane is an explicit mode scoped to the current context and suspended
by manual navigation. No automatic Stop gate.

### Managed children and deferred options

Managed children (reviews/analysis) are excluded. Deferred: Claude
`lspServers` shim (E3, off the critical path), speculative `checkEdit`, agents
writing through buffers, LSP multiplexing.

## Measurement and rollout gates

Correctness first: false-clean and missed-regression rates against captured
`check@1`/test outcomes (evidence for their inputs, not universal truth),
including cold indexes, dependent files, concurrent writes and server failure;
abstention and useful-result coverage alongside. Then edit-to-evidence p50/p95,
resource use, tokens per task and fix-turns per broken edit. Telemetry records
counts, states and latencies only, never diagnostic text.

## Phasing

| Phase | Scope | Gate |
| --- | --- | --- |
| P0 | Machine watcher and Observer, executor receipts, standing continuation, analysis-frontier protocol extension (document evidence done: Zed 1.22.0) | conformance on the pinned pair; gap/reset fixtures |
| P1 | `cowboy.code.search/outline/symbols`; `code.diagnostics/summary` over existing safe observations only; badges | non-projecting resolver and scope fixtures; no acquisition |
| P2 | Acquisition gates (scoped fences, Unknown recovery, budgets) then automatic acquisition; baselines and matching | availability tests with lost replies and restart |
| P3 | Navigation pull after owned-navigation acceptance | that route's acceptance |
| P4 | Claude push, then Codex push | native delivery conformance (held fetch, subagents, TaskStop, injection) |
| E3 | `lspServers` shim experiment | same gates |

## Ownership and deletion triggers

The Machine Observer, executor and Controller own the service; provider
Plugins own only their delivery adapters. Delete the Codex adapter if Codex
ships native post-edit diagnostics that can target a remote LSP; delete the
Claude push if Claude Code consumes a remote language server natively with
equal freshness guarantees.
