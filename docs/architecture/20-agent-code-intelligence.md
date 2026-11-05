# Agent Code Intelligence from the Code Review Engine

**Status: design proposal r3 (2026-11-05), not implemented.** r3 folds in two
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

### Acquisition: gated

The adapter has a process-wide admission fence: an unresolved native Open,
navigation or synchronization refuses every new open on that Machine, with no
recovery path yet (`docs/plugin-native-open-once.md`,
`docs/plugin-owned-navigation.md`, `docs/plugin-buffer-sync-owners.md`), and
reopening a path can adopt a shared native buffer. Automatic paths would
multiply Unknown outcomes. Therefore **no automatic acquisition** (pull, badge
or push) until all of:

1. fences scoped to the native-ID alias set of the target, never to pathname
   equality;
2. independently authorized Unknown recovery and cleanup tied to exact native
   process identity, surviving Controller restart and worker replacement;
3. proven isolation from and priority below interactive Code-pane acquisition;
4. the per-Machine process budgets of [19](19-code-review-engine.md).

Until then agents receive only observations that already exist safely
(buffers a Code pane holds, structure tags, `check@1` results), or
`unavailable`.

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
| `code.outline({path})`, `code.symbols({query})` | structure tags, refined by the language engine when available |
| `code.diagnostics({paths?, compare?})` | per path: state, coverage, problems with source lines, counts |
| `code.summary()` | observed-scope counts with coverage |
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
| P0 | Machine watcher and Observer, executor receipts, standing continuation, analysis-frontier protocol extension | conformance on the pinned pair; gap/reset fixtures |
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
