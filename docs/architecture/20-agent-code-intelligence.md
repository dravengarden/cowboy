# Agent Code Intelligence from the Code Review Engine

**Status: partially implemented; remaining gates below (2026-11-07).** r3 folds in two
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
- Linux inotify continuity is **event-stream-only** coverage. Its documented
  blind spots include memory-mapped writes and remote network-filesystem
  modifications; an empty reconciled page is not evidence of unchanged file
  contents or complete project inputs. Native source identities and analyzer
  frontiers remain separate requirements. See the
  [Linux inotify limitations](https://man7.org/linux/man-pages/man7/inotify.7.html).
  Observer pages carry this coverage explicitly, including reset and gap pages.
  The bounded Linux source watches regular-file inodes as well as directories
  to observe external hard-link writes, follows no repository symlinks, and
  invalidates its window on structural ambiguity, root/mount changes, queue
  overflow or resource exhaustion. Structural reconciliation cannot make a
  comparison from before the gap valid again.
  The native worker library reserves two roots per resident owner, bounds its
  request queue and retains capacity through actual collection/descriptor
  cleanup. Revocation wakes cancellation; a timed-out close cannot release a
  still-running scan's reservation. Failed idle scans do not retry forever.
  The private protocol-42 integration binds Machine requests to the grant
  captured before async dispatch and clones the original pinned root handle.
  A bounded process-local receipt registry prevents uncertain Start retries,
  Close-before-Start and completed observations from creating another worker.
  Close reports `closing` until collection and descriptor cleanup are confirmed.
  Receipt exhaustion refuses admission instead of evicting replay fences.
  Controller diagnostic demand captures the original Session, worker authority,
  connection and language-server policy; passive summaries do not prolong it.
  Invalid or five-minute-idle continuations retire through their original
  connection. Pending cleanup prevents a fresh instance for that parent;
  cleanup visits are bounded and rotate past unconfirmed outcomes. Transport
  loss abandons the old route without claiming native thread exit or redirecting
  cleanup. The exact protocol-42 candidate passed native Codex/Claude execution,
  recovery, Provider coexistence, logs and connected Code acceptance
  ([receipts](../experiments/code-observer-worker-pin-2026-11-07.json)).
  Hawk Machine and Controller activation of `84b6e45bf` both recorded
  `succeeded` / `committed`. Running executables matched the exact releases;
  all four Machines reconnected. A temporary Go probe returned its exact
  content hash, `document_current` and the expected `UndeclaredName`; Machine
  fd inspection separately confirmed an inotify observer of this Session root.
  The probe was removed after verifying its hash. Two retained Hawk Sessions
  underwent managed generation cutover and returned to `running`, unpaused
  with empty queues; their old worker PIDs were not preserved. A third earlier
  worker PID had also ended, without evidence here attributing its exit.
  Event pages are not yet analysis baselines or model delivery authority.
- **Trust lifetime.** Agent diagnostic reads capture the Session and the
  language-server policy lifetime before asynchronous work. A policy change,
  including off/on before the result arrives, discards old diagnostic content
  and counts. Folder rename preserves trust; placement, order, hierarchy and
  relevant default changes invalidate it. This is a read fence, not a worker
  grant or a Machine observation continuation.
- **Executor receipts** are latency accelerators: binding incarnation, turn,
  tool identity, monotonic sequence, operation kind and before/after content
  identities, emitted after durability. ACP `locations`/`diff` are unverified
  hints only. Intermediate versions are superseded; delete/recreate is a new
  identity.

  A Controller-side hint is implemented first; the durable keeper receipt is
  deferred to a Machine maintenance release. Every native call already passes
  through the Controller's execution `forward`, which now records each
  completed `fs/writeFile` (including a completion later observed by its
  operation ID): the absolute target path and the SHA-256/byte count of its
  LF-normalized UTF-8 text, the native mirror's identity. Records are bounded
  Controller memory (4,096 writes, 256 pending, 4 MiB inputs) with a
  monotonic sequence; a restart discards them. `code.post_edit` reports
  `write: {observed, matches_analyzed}` per file, true when the analyzed
  snapshot is exactly the last observed write. There is no turn or tool
  identity, failed/unknown outcomes are not recorded, and shell commands,
  Claude's buffered apply helper (`process/start`) and other writers are not
  seen, so absence proves nothing. Which executor method Codex `apply_patch`
  uses is not yet verified against the pinned executor.

  Production check on 2026-11-08 (Controller `b9dc4ff51`, transaction
  `1794144305234690764-b9dc4ff51a24`): neither agent's actual edit path is
  seen yet. A Claude 3.20.27 Write of a new Go file returned diagnostics
  without `write`, consistent with the buffered apply helper. A Codex 3.5.21
  create-and-edit received its steered row (`injected`, `introduced: 1`) also
  without `write`, so its patch application does not use `fs/writeFile`
  either. The hint therefore covers only direct `fs/writeFile` callers until
  the helper's `process/start` result and Codex's patch method are observed.

  Follow-up: that check was invalidated by a Controller bug. Native callers
  send `file://` URLs and the first filter accepted only plain absolute paths;
  URLs are now parsed. After the fix a Claude Write still showed no `write`,
  because file buffering routes Write/Edit through the apply helper. The
  Controller now also follows an inline helper request (`process/start` argv
  `apply --request`, path and expected after-digest) by process ID through
  that process's `process/read` stdout and records the write only for an
  `applied` outcome with exactly the expected digest. The helper digest is of
  the written bytes, so CRLF files never match the LF-normalized mirror.
  Requests large enough to travel in a file are not observed. Helper reads that
  wait are followed through `Observe` by operation ID, and a helper is dropped
  only once its output pipe closes (exit can precede the last output).

  Production evidence with temporary path/route logs (since removed),
  Controller `713486b22`: a Codex 3.5.21 create-and-edit wrote through
  `fs/writeFile` with `file://` URLs. Both writes were observed at the exact
  paths the post-edit lookup used, and the steered row carried
  `write: {observed: true, matches_analyzed: true}`. Claude coverage is not
  achieved: for a Claude 3.20.27 Write the Controller saw the helper start and
  one helper read with no output, but the reads that carried the outcome never
  reached this observer, so no `write` was recorded; the cause is unresolved.
  Claude rows therefore never carry `write` today, which proves nothing.

On 2026-11-07 the Controller activated
`1ad869c5fed6f033583b75a5c2db09869b60b678` through transaction
`1794056365894580054-1ad869c5fed6` (`succeeded` / `committed`). The full pinned
gate passed, including the six native observer-worker lifecycle fixtures and
the off/on diagnostic-result regression. The clean Nix artifact's source
receipt and the running Controller executable matched the candidate. Both
existing ACP workers retained their PIDs, start times, executables and argument
digests; all four Machines remained online, health returned `ok`, and the SPA
returned HTTP 200 with `no-store`.

A real MCP diagnostic read of a temporary 66-byte Go file returned the expected
`UndeclaredName` finding from gopls, `document_current`, and matching source
SHA-256 `f4b93bc006a6002314d7e446062ffe9dba1ca3c83cd21b6c7a9cdf71879ffd44`.
The file and empty directory were removed after verifying that digest. The
policy-revocation and cancellation fault cases are isolated fixture evidence;
production policies were not toggled for this acceptance. This release enables
the Controller read fence only: the Machine observer library is not yet wired
to a standing continuation or enabled as production background watching.

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

Zed 1.32.0 additionally retains a process-local recovery
receipt on each owned Open before its first await. Admission binds that receipt
to the original native attempt before dispatch; a later attempt cannot satisfy
or replace it. Query may retire an abandoned lease when no Open was admitted,
the original peer confirms failure/non-dispatch, or the original cleanup was
confirmed. It never sends another Open or starts orphan cleanup. An acquired
orphan without a dispatched cleanup remains Unknown; pending/unreachable native
operations retain their fence. Once independently authorized recovery has
confirmed cleanup, the old lease can retire even if a later attempt is pending,
without querying or clearing the later attempt. This is an owned-reference
recovery improvement, not Machine restart persistence or legacy maintenance.
On 2026-11-07 Hawk installed this release from
`941b6b790dd8fddace7467d67260f71c6424b97e`, composite digest
`64fbaff7169bbe9b82f89cd222ff46bfb19bf0abd11205f9b08e52148ed44dba`,
through `hawk-zed-1-32-0-open-recovery` (completed/applied). The complete
gate passed, including 159 adapter tests. The exact static native pair proved
that querying a real failed Open retires its original lease without replay,
while independently prepared replacement and known-buffer owners still work.
Signed installation/drain, actual active/recovery Catalog readers and all
three published URL digests passed. A production MCP probe retained matching
content/diagnostics initially and `stale`/`changed` after a disk edit; its
summary preserved the reason and its temporary file was removed. Controller
PID 1875594 and both existing Hawk workers were unchanged, health and SPA
checks passed, and all four Machines were online. Unknown failure injection
is isolated native-fixture evidence, not a production-session experiment.

Zed 1.33.0 adds prepared release references for confirmed
automatic legacy owners. The Machine retains the original runtime and reference
before sending Execute; after uncertainty it queries that reference, never a
replacement runtime or owner. A preparation is bound to the legacy pin's
incarnation and native ID. Admitted native Close fences legacy aliases; only
the same native Close receipt can complete it. Query never starts a Close.
Released references retain their original result until explicit retirement;
Retired is a separate state and is not release evidence. Effect-free
preparations expire after 30 seconds; unknown and completed records do not.
Both adapter records and Machine retirement permits are bounded to 1,024.
Machine maintenance rotates at most four holdings and four retirements per
pass, with two-second private exchanges. Cancelled retirement observations
return their original reference and permit to the queue. Older Plugin
generations retain the previous one-use Close and its deadline; an uncertain
legacy fallback is not replayed. Generic Code RPC cannot enter this internal
release protocol. This does not recover uncertain legacy Open/EnsureWorktree,
Machine-process loss, or a Controller receipt lost after the Machine completed
and forgot its holding.

The exact `db286dddca7035b01708c742f782c66ce3a8b54d` release passed the full
gate, 163 adapter tests (two ignored), 84 focused Machine tests (four ignored),
and the immutable native pair. The latter discarded an actual Close reply,
queried its original reference, reopened the same owner, and proved that the
old Execute preserved the replacement. Signed install/drain, both actual
Controller Catalog readers and all published URL hashes passed. Hawk activated
the resident Machine in transaction `1794031492207402104-db286dddca70`, retaining
worker generation `worker-39e0360cdcbff26cfa61`; the two existing worker PIDs,
executables and arguments and Controller PID were unchanged. Installation
`hawk-zed-1-33-0-legacy-release` completed/applied, and inventory reports 1.33.0
active with composite digest
`sha256:a3c81fb50d05b536ef4d4b1fd9090c199a6af9059cbe37cea271fe1f0da50b15`.
A temporary production Go probe matched its exact source SHA and returned
`UndeclaredName`; a passive summary after 320 seconds reported `not_open`.
The probe was removed after checking its bytes. That summary alone is not
original Close receipt evidence; lost-reply recovery was tested in isolation.

The Controller/Machine release separates Controller observation-cache
retirement from native resource release. Repeated acquisitions request
`refreshOnly`, which renews only a confirmed owner on its original live route;
it cannot Ensure/Open or recreate an idle-reclaimed owner. Unknown, released,
foreign or replaced owners fail without a fallback Open. The Controller retires
explicitly unavailable refresh observations and completed/cancelled cleanup observations, with
nonce checks and pending-opener accounting. A later acquisition uses a fresh
nonce and remains subject to Machine quotas, which still count all old unknown
owners. Forgetting a Controller observation is not evidence of native release.
This also bounds Controller cache retention after a lost Close reply without
claiming to recover that lost result. Uncertain refresh transport outcomes keep
the original nonce for harmless retries without refreshing its idle deadline.
Renewal itself does not sample resources or select a runtime: the existing
owner remains charged, and only new acquisitions need resource admission.
An original Open that is still unconfirmed returns an error rather than the
absent-owner reply, preserving the Controller's original idle Close attempt.
Hawk activated source `649f034c6aec034b0e18f4aae715fceed342e7ce`: Machine
transaction `1794035146041326035-649f034c6aec` and Controller transaction
`1794035632700203074-649f034c6aec` both succeeded/committed. Health and SPA
returned HTTP 200, the SPA remained `no-store`, all four Machines were online,
and both existing Hawk worker PIDs, executables and arguments were unchanged.
The first post-restart probe returned `not_open`; the next three returned
`document_current` / `matching` with the same native instance, buffer and exact
source SHA. Warm round trips were 259 and 270 ms in this small observation,
not a performance benchmark or proof that no native Open was issued. The latter
property is covered by the refresh-only implementation and isolated fixture.
After more than five idle minutes, passive summary reported `tracked: 0` and
`unobserved`, accepting Controller observation retirement rather than native
Close completion. The probe was removed after verifying its exact SHA.

Zed 1.34.0 adds the missing unresolved-Open check to the common
native Close admission used by legacy, owned-buffer and navigation releases.
An unobserved Open can return an existing native ID, so the last native owner
must survive until that original Open is resolved. Releasing a shared local
owner without native Close remains allowed. Recovery of an already dispatched
Close still queries only its original receipt and does not clear another
Open's fence. Legacy release preparation also refuses while Open is unresolved.
A regression failed against 1.33.0 and passed after this guard. The full gate,
166 adapter tests (two ignored), immutable native pair, signed install/drain,
Catalog readers, signature and published URL hashes passed. Installation
`hawk-zed-1-34-0-open-close-fence` completed/applied and inventory confirmed
1.34.0 active, composite digest
`sha256:f568d3e395c52c83184243f8c584294ab8c21fea3afd9b4f90c9214ecaeb1e07`.

Zed 1.35.0 removes the effect-free preparation's time expiry. A
preparation remains valid while its exact original legacy pin exists; a lost
Prepare reply returns the same reference on retry, including at capacity.
Only preparations whose original pin disappeared or changed can be pruned.
Unknown and completed receipts still require their original recovery/retirement
path. This prevents delayed execution from permanently stranding a Machine
holding behind an expired reference. The immutable 1.34.0 adapter failed the new
31-second/discarded-Prepare-reply regression by replacing reference 1 with 3;
the exact 1.35.0 artifact passed the same test with its original reference.
Source `3056f09dd5dfde8819f0d126934320b9da82035b` passed the full gate,
166 adapter tests (two ignored), immutable native pair and signed installation/
drain. Both actual active/recovery Controller Catalog readers accepted the
publication; independent production signature and all public URL hashes passed.
Installation `hawk-zed-1-35-0-prepared-release` completed/applied with revision
`installation-85115af72b7fc502386c275092b56e1ac0634fd06922d10baa8b205d0e0627ca`.
Hawk inventory reports 1.35.0 active, composite digest
`sha256:f00af7bdef6261b0c2cd6bdcbf9c3d74f034f1f6503fdda76ab3d633cb406734`.
The production Go probe returned `document_current` / `matching`, exact SHA
`e9d99d3a5185fc16770f7794a78c25ed715d2a2f0060371a63c4eab7f044ce5b`, and
`UndeclaredName`; it was removed after checking those bytes. Health remained
HTTP 200 and both existing Hawk workers retained their PIDs, executables and
arguments. No Controller or Machine restart was needed for this Plugin update.

Zed 1.36.0 subsequently accepted the bounded native Open journal correction:
evicted results are `FORGOTTEN`, pending requests are never evicted, and reused
or decreasing IDs cannot admit another Open. The exact native regression,
production installation and limits are recorded in
[single-use native acquisition](../plugin-native-open-once.md).

Machine revision `1d47b85e08f65f19d2fa28c2818647b692c129eb` tracks EnsureWorktree confirmation separately from
Open confirmation for nonce-scoped automatic owners. Once the worktree is
confirmed, a lost Open reply may use the
original prepared-release protocol; the Plugin must still prove that original
owner exists. Maintenance can attempt that same recovery after idle. Unknown
EnsureWorktree retains its reservation and refuses both prepared release and
legacy Close, since an empty buffer reply cannot account for the worktree effect.
Old Plugins' idle fallback still requires confirmed Open. Refresh continues to
require confirmed Open and never treats release eligibility as readiness.
This does not recover an unfinished native Open or an unknown worktree effect.
The complete pinned gate passed, as did the installed native fault-injection
fixture that discards the successful Open reply and releases that original,
nonzero buffer ID. Its resource admission is synthetic; it does not prove host
resource admission. Hawk activated the host release on 2026-11-07, transaction
`1794040894952929953-1d47b85e08f6` (`succeeded` / `committed`), retaining
`worker-39e0360cdcbff26cfa61`. Both existing worker PIDs, executables and arguments,
and the Controller PID, were unchanged. Health, version and SPA checks passed;
all four Machines were online and Zed 1.36.0 remained active. A temporary Go file
returned `document_current`, matching source SHA and the expected `UndeclaredName`
diagnostic; it was removed after verifying its exact SHA. Production did not
inject reply loss, and this acceptance does not cover Machine-process loss.

Zed 1.37.0 extends prepared release to an unfinished legacy Open.
Its pin retains that exact acquisition's recovery evidence before native I/O;
repeating the unresolved Open cannot replace it. Preparation is effect-free.
Execution queries the original native Open and, if necessary, releases its
unowned result through one original native Close. Pending or forgotten Open
results keep the reservation. Cleanup admission marks the prepared reference
unknown; subsequent queries only observe the same Open/Close receipts, without
replaying either effect. A confirmed no-effect Open can release without Close.
The native fixture verifies an actual uncommitted Open and a fresh share after
its release; synthetic transport tests additionally discard the cleanup reply.
This remains process-local and does not recover unknown EnsureWorktree effects.
Source `a1aee90dc2196245bc26fc422f4ff670665d3171` passed the complete pinned gate,
the exact static 1.37.0 adapter / 1.14.0 server pair and temporary signed
installation/drain conformance. Actual Controller active/recovery Catalog readers
accepted the bound release; its signature and public package/runtime hashes were
verified independently. Hawk operation `hawk-zed-1-37-0-unfinished-open` completed
with Machine receipt `applied`, installation
`installation-77598a26bb6c52c2ba125a796647c60b143502a1a662afdf984e8f918a437163`,
composite digest
`sha256:8def344c6232e521f1860ac2b52ad7ef1f3b3c8d2e5a5e910940f15fc56e3b79`.
Zed 1.36.0 remains the rollback generation. Both existing worker PIDs,
executables and arguments, and the Controller PID, were unchanged; health,
version and SPA checks passed and all four Machines were online. A temporary Go
probe returned `document_current`, matching source SHA and the expected
`UndeclaredName`, then was removed after verifying its exact SHA. Production did
not inject reply loss; the fixture's synthetic resource admission does not prove
host resource admission, and none of this accepts Machine-process-loss recovery.

Zed 1.38.0 prepares an automatic acquisition reference before
EnsureWorktree, then retains the original native Add, Trust/Restrict and Remove
request identities. The native journal reports Add completion only after store
registration; the upstream early response is insufficient. Pending and uncertain
entries retain their fences, bounded history eviction reports Forgotten, and a
replacement connection cannot supply the original result. Queries observe only;
absence from the journal is not a no-effect receipt, even after Ping succeeds.
Explicit release continues the original buffer and worktree cleanup in order.
The last automatic owner removes the native worktree only when no interactive
worktree lease or buffer owner remains. A captured incarnation and positive
retirement evidence prevent an old request from closing a later replacement.
The prepared buffer's worktree incarnation must match the captured acquisition
before Open; replacement between Ensure and preparation is refused.

Machine saves the prepared reference before execution, keeps uncertain worktree
acquisitions charged, and can reclaim them after the existing idle interval.
Its bounded retirement queue retains the original runtime and capacity permit
until retirement is confirmed or that runtime exits. Only an explicit old-codec
unknown-variant refusal selects the older release path; unsupported native
journals, malformed support replies and transport errors do not. These private
requests are denied by generic Code RPC. Deploy the compatible Machine host
before installing this Plugin. This is a process-local protocol, not durable
Machine restart recovery or project freshness.

Source `7c8a73a493104ca214491e8c83934bd019393a61` passed the complete pinned
gate, including 175 adapter tests, and the exact static 1.38.0 adapter / 1.15.0
server pair passed native and signed installation/drain acceptance. Actual
native Add, Trust and Remove replies were discarded in isolated fixtures;
queries recovered only original outcomes, and retired worktree IDs refused
new buffer opens. Machine fixtures additionally cover lost prepare/execute/
release/retirement replies, foreign references, unsupported native recovery,
unfinished acquisition cleanup and receipt-capacity exhaustion before ownership.
The temporary installed fixture discards the actual successful acquisition
reply and confirms original cleanup plus maintenance retirement. Its synthetic
resource admission does not prove host resource admission.

Hawk activated the compatible Machine host first, transaction
`1794048124208384270-7c8a73a49310` (`succeeded` / `committed`), retaining
`worker-39e0360cdcbff26cfa61` and both journal writers disabled. Actual Controller
active/recovery Catalog readers accepted the signed publication; the signature
and all public artifact hashes were verified independently. Operation
`hawk-zed-1-38-0-worktree-acquisition` then completed with Machine receipt
`applied`, installation
`installation-38f3a3e9813e7c3bd9a66465e54f6e97257a4fc462a8173c12bdfee8dc6a6646`,
composite digest
`sha256:e204b7f6179f61accd756f3a02b77028f8a846aee76b500b6a372dd48d33d81e`.
Zed 1.37.0 remains the rollback generation. A temporary Go probe returned
`document_current`, matching independently computed source SHA and the expected
`UndeclaredName`, then was removed after verifying its exact SHA. Both existing
worker PIDs, executables and arguments and the Controller PID were unchanged;
health, version and SPA checks passed and four Machines were online. Production
did not inject reply loss, and this does not accept Machine-process-loss recovery.

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

Agent diagnostic projection requires canonical native identities. An explicit
provenance that conflicts with the response's bounded buffer snapshot refuses
the whole file observation, including counts. Missing snapshots cannot establish
provenance equality; legacy diagnostic text remains historical evidence.
This identity check does not establish a text-version mapping or causality.

Zed 1.29.0 preserves the native diagnostic code and
provenance (observation-stream instance, buffer ID and incarnation, server ID, source kind and
registration) for agent language observations. Native u64 IDs are decimal
strings so JavaScript cannot round them. A replacement stream gets a new
instance; message or severity changes do not create a new provenance identity.
The agent projection adds the complete end position and returns unavailable
identity as null, including over-budget codes/registrations; it never truncates
an identity or derives one from message text. Owned-read protocol one retains
its existing closed diagnostic shape. These fields are inputs to future
content-matched comparison, not a baseline or proof that an edit caused a
diagnostic.

Hawk installed signed Zed 1.29.0 on 2026-11-07 from
`b54c20327552a414b528339510331e60cf8e511a`, composite digest
`7d6b7ac044e6b3a1fef3a73c14abd67c24a9057d364acf60f898a24f20258f35`.
The installation operation `hawk-zed-1-29-0-diagnostic-provenance` completed
with an applied Machine receipt. The exact static adapter/server passed native
navigation, diagnostic identity, old owned-read compatibility and signed
installation/drain fixtures. Active and rollback Controller readers accepted
the exact publication; all three published artifact URLs matched their digests.
The integrated Controller `8d6b390f19c697d495177f64a55e62437276e957` was
activated by a concurrent task, transaction
`1794021841732582984-8d6b390f19c6` (`succeeded` / `committed`); this task
verified that receipt instead of restarting it again. A temporary file in this
session's worktree produced a real MCP diagnostic with code `UndeclaredName`,
complete end position and native provenance, with gopls `document_current`.
The fixture was removed afterward. Existing Worker PIDs, executables and
arguments remained unchanged; four Machines were ready and health/SPA checks
passed. This verifies the deployed metadata path, not baseline matching,
project freshness, automatic observation or push delivery.

Zed 1.30.0 adds `bufferSnapshot`: the original mirror
instance, buffer incarnation, exact text clock and SHA-256/UTF-8 byte count of
its LF-normalized text. The same revision check protects diagnostics and this
snapshot across asynchronous language queries. Language responses now return
the version actually queried, rather than the original Open version. Empty
diagnostic sets can carry this text identity too; missing or over-budget
identity remains unavailable. Owned-read protocol one stays unchanged.
This identifies the native mirror, which may differ from disk; it is not an
immutable baseline capture, an analyzed-input frontier or causal attribution.
Hawk installed the signed release from
`4b9bfee86af161d3b8b4dd827638fa5cbd3a4675`, composite digest
`6256172e22e016268d90392458a71610c3c57d68636f6faa4146bdaeb1b4dd03`,
through operation `hawk-zed-1-30-0-buffer-snapshot` (completed/applied).
Controller activation `1794024532993336903-4b9bfee86af1` succeeded and
committed. The full pinned gate, exact static native pair, signed install/drain,
active/recovery Catalog readers and published URL digest checks passed.
An actual MCP read of a temporary CRLF/Unicode file returned the independently
computed LF-normalized SHA-256 and 74-byte length, together with its native
version and diagnostic provenance. The fixture was removed; existing workers
were unchanged and health, SPA and all four Machine readiness checks passed.

The subsequent disk-edit probe exposed a remaining distinction: the native
mirror and its `document_current` analysis still described the old text after
the file changed. The new snapshot made that mismatch observable, but this
release neither reloads the buffer nor compares it with the current source.
Source mismatch must downgrade the agent-facing state before baseline capture;
it must not borrow synchronization authority or promote file quiet to freshness.

Zed 1.31.0 checks that distinction on the execution
Machine. Its bounded reader resolves a regular file beneath an opened worktree
directory, refusing symlinks, mount crossings and non-UTF-8/oversized inputs.
It compares two bounded reads and file identity/metadata, using the same line
ending normalization as the native mirror. There are at most two actual reads
in flight; timed-out blocking jobs keep their permits until they exit. The
language snapshot is revalidated after this observation. `sourceState` is
`matching`, `changed` or `unavailable`: changed source makes the agent result
`stale`, and unavailable source evidence cannot produce `document_current`.
Per-server analysis and the captured native snapshot remain visible. No file
is saved or reloaded, and source equality is not a project input frontier or
an automatic synchronization grant.

On 2026-11-07, Hawk installed that release from commit
`998f199b52c12d27ade6439c4fb7b34e0bbf5c7b`, composite digest
`722f143bbfbe5889a3a55196177ed4260a114f2d87f663b00ee536800bf6e64d`,
through `hawk-zed-1-31-0-source-state` (completed/applied). Controller
activation `1794027421589112874-998f199b52c1` succeeded and committed.
The full gate, 155 adapter tests, exact native pair (including a real bind
mount refusal), signed installation/drain, active/recovery Catalog readers
and all three published URL digests passed. Actual MCP reads first matched
a temporary CRLF/Unicode Go file's independently computed 75-byte normalized
content, then reported `stale`/`changed` after a disk edit while retaining
the old native snapshot and diagnostic. The passive summary retained that
reason. The source bytes were unchanged by observation and the fixture was
removed. Health and SPA checks passed, all four Machines were online, and
both existing Hawk worker PIDs and arguments were unchanged. This accepts
source mismatch detection, not watcher frontiers or automatic baselines.

- Baselines are immutable, content-matched captures at a read or pre-write
  boundary (the facade and executor see pre-write content). Without one the
  comparison is `unknown`.

  The read boundary is implemented. Each explicit `code.diagnostics` read that
  is `document_current` and passed its trust fence captures the complete
  observation (at most 200 diagnostics) bound to the snapshot's text SHA-256
  and byte count, keyed by the exact Session code scope and path. When a later
  read sees different text, the previous capture becomes the baseline and the
  reply's `comparison` names it (`against`). The same text never becomes its
  own baseline. Comparisons are `unknown` without a baseline, for a reply that
  is not `document_current`, after buffer replacement (a different native
  stream), when the server set changed or a capture exceeded its bound.
  Passive `code.summary` neither captures nor compares. Records are
  Controller memory, bounded to 64 files per scope and 512 overall (least
  recently observed first); a restart discards them. Pre-write capture waits
  for executor receipts, so a read boundary may precede several edits.

  The read boundary shipped in `02d1da0c3`; Hawk's Controller runs it through
  the concurrent activation of descendant `270f49cd9`, transaction
  `1794123221502657538-270f49cd9f4c` (`succeeded` / `committed`), whose running
  executable matched the active release. A real MCP read of a temporary Go
  file returned gopls `document_current`, its exact 62-byte SHA-256, the
  `UndeclaredName` finding and `comparison: unknown/no_baseline`. After a disk
  edit, repeated reads stayed `stale` with `sourceState: changed` and the old
  snapshot, so the comparison stayed `unknown/not_current`. The fixture was
  removed after verifying its hash. In production a comparison therefore needs
  the native mirror to observe new text: until an agent-owned reload on source
  change (or a write receipt) exists, disk edits by tools do not produce one.
  Reopening a buffer is reported as `buffer_replaced`.

  Zed 1.39.0 closes that gap for agent-only buffers. `bufferLanguage` accepts
  `refresh: "agent_owned"`; the adapter reloads the mirror through the
  existing bounded native reload route only when every owner of the buffer is
  an automatic agent acquisition (owner IDs are never reused), no
  synchronization reservation or unresolved close exists, and its bounded
  source reader reports `changed`. The active-buffer read lock is held through
  the reload, so no owner joins or leaves meanwhile. Outcomes are
  `not_needed`, `reloaded`, `refused` (another owner, fence or native
  size/binary/symlink refusal, mirror unchanged) or `unknown` (unobserved
  within 5 s; never retried). Nothing is saved, opened or synchronized. When an
  explicit `code.diagnostics` read of a file the agent already holds sees
  `changed`, the Controller sends one refresh and, after `reloaded`, waits up
  to 4 s for analysis of the new text before answering. The Machine already
  classifies `bufferLanguage` as passive, so no Machine release is needed; an
  older adapter rejects the unknown field and the original observation is
  kept. A file a Code pane also holds stays `stale`.

  Source `680d83d46` passed the complete pinned gate. The exact static 1.39.0
  adapter (`5c1e8c71…`) / unchanged 1.15.0 server (`b2171b6e…`) passed native
  sync/navigation conformance, including a real exclusive-owner reload,
  shared-owner refusal and unchanged source through the distributable
  adapter's socket, and signed install/drain. Controller transaction
  `1794127149954701623-680d83d46f3f` succeeded and committed with the running
  executable matching the release. The signed package (composite
  `sha256:ccfe12562186857759f130aef1aa667d47f40149c3ffa50c7b37f833a05d641d`)
  was published with all three URLs matching their digests; operation
  `hawk-zed-1-39-0-agent-refresh` completed with Machine receipt `applied`
  (installation `installation-310bda29…`), 1.38.0 remaining the rollback
  generation. A real MCP read of a temporary Go file returned
  `document_current`/`no_baseline`; after a disk edit, one read (273 ms)
  reloaded the agent-only mirror and returned the new SHA-256 with gopls
  `document_current` and `comparison: complete` (introduced 1, persisting 1,
  two ambiguous `UndeclaredName` candidates). The serving adapter's executable
  hash matched 1.39.0. The fixture was removed after verifying its hash.
- **Hunk overlap** orders diagnostics within the budget (`in_edit` before
  `elsewhere`); it never asserts that an edit caused a diagnostic and never
  replaces the baseline comparison.
- Identity (source, code, anchor) is separate from mutable attributes
  (severity, message); severity escalation is a change. Matching is
  multiplicity-aware with versioned anchor mapping; without `code`, provenance
  and ambiguity are preserved rather than hashing messages.

  The implemented matcher keys identity on the native server, source kind,
  registration, source and code. It has no anchor mapping yet: per identity it
  pairs instances most severe first, counting `persisting`, `escalated`,
  `introduced` and `resolved`. Moved instances persist. New instances of an
  identity the baseline already had are listed as `ambiguous` candidates (up
  to ten), since any of them may be the new one. Diagnostics without a code or
  bounded provenance are counted as `unmatched` and make the comparison
  `partial`. No field claims an edit caused a diagnostic.
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
| `code.diagnostics(paths)` | implemented: diagnostics and counts with a per-file state from per-server analysis-frontier evidence (`document_current`, `stale`, otherwise `last_observed`), or `not_open`/`restricted`/`refused`/`unavailable`. A file no Code pane holds is acquired for the agent (`acquired: true`, see Acquisition) and waited on for up to 4 s. Each file carries a read-boundary `comparison` (see Baselines) |
| `code.summary()` | implemented: passive counts over this session's tracked files (`coverage: agent_observed_files`), without acquisition or idle-time refresh. No usable observations means `counts: null`/`unobserved`; `partial` counts exclude unavailable files. Counts are never project totals, and Controller restart discards the tracked set until files are read again |
| `code.hover({path,row,column,sha256})` | implemented over already-open Session buffers: zero-based UTF-16 position and exact content hash, original policy/route fences, identical bounded native snapshot before/after. Five-second read bound; at most 8 blocks / 2,000 text characters with explicit truncation; escaped envelopes over 12 KiB are refused. Never acquires a buffer or destination; stale locations are refused |
| `code.definition/references/implementations(locator)` | after owned-navigation acceptance only |

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

  Implemented (Claude Plugin 3.20.30, component release 3.99.0): after the
  main agent's successful Edit, Write or NotebookEdit of a file inside the
  target workspace, the Mod returns the write result unchanged and schedules
  one observation. It runs the session's own `cowboy mcp` on the target
  through the existing hook route (the keeper injects the gateway context; no
  credential reaches the Provider runtime) with one `cowboy_execute` call of
  the read-only action `code.post_edit`. That action is the explicit
  `code.diagnostics` read (acquisition, agent-owned refresh, read baselines)
  reduced to at most 10 errors, 5 warnings and 2,000 message characters,
  with `deliver` true only for new or escalated diagnostics against a
  content-matched baseline, or current errors without one. The Mod appends
  the result as its own `system-reminder` row, with `<`, `>`, `&` and line
  separators escaped inside the JSON so repository text cannot close the
  fixed presentation. Observations run one at a time in batches of up to five
  files; a file edited again meanwhile is dropped from the earlier result.
  Subagent edits, shell-made changes and Code-pane-shared files (which stay
  `stale`) produce no row. Project language servers being on is the per-session
  opt-in: otherwise the read is `restricted` and nothing is delivered. Each
  bridge exchange holds at most one second; the write itself never waits.
  Delivery identity states, `context_cleared` dropping and watcher-triggered
  coverage remain open.

  Source `e64747ba8` passed the full gate, `claude-remote-check`, and the
  packaged `execution-worker-conformance` (accepted, 97 checks, unchanged set)
  and `execution-managed-conformance` (accepted, 7 checks) with the exact
  rebuilt artifacts. The endpoint-close fixture's 15 s deadline was shorter
  than an idle event poll's 20 s bound; the post-edit observation exposed it
  and the deadline is now 25 s. Controller transaction
  `1794134057192471504-e64747ba8734` succeeded and committed. A direct
  production `code.post_edit` returned `deliver: false` for a clean file and
  `deliver: true` (`introduced: 1`, `undefined: missingName`) after the edit.
  Claude Plugin 3.20.30 (composite
  `sha256:4b388ab6f31acacdbc8df80ff10e9a0db350cc7bb0947eb9bbf592c136137e5d`)
  was published with all five URLs matching and installed on OVH by
  `ovh-claude-code-3-20-30-post-edit` (`applied`; 3.20.27 is the rollback
  generation). A new 3.20.30 session wrote and then edited a Go file; its
  model had the row with `comparison: complete`, `introduced: 1` and the
  `UndeclaredName` message. It answered its own turn before noticing the row,
  so delivery is next-request, not synchronous. The session was trashed.
- **Codex.** Existing research proves app-server-owned command hooks only, not
  project-hook exclusion (`docs/experiments/codex-native-hooks-research-2026-10-05.json`).
  A hook is accepted only if conformance proves exact Cowboy-owned hook hashes
  are trusted while project hooks are refused under the session's
  `trust_level: "trusted"`, and that it reaches the original execution binding
  without credentials in the runtime. Otherwise Codex push is delivered from the
  Cowboy side at the next safe point.

  Implemented from the Cowboy side; no Codex hook is used. A Controller task
  watches Codex-family sessions with Project language servers on that are not
  managed children. When an ACP `edit` tool call completes, its diff and
  location paths (unverified hints) inside the execution worktree choose the
  files, and the same `code.post_edit` read runs after a 300 ms settle. The
  turn is captured when the edit is observed; a `deliver: true` result is
  steered into that turn at its next checkpoint, so an ended or later turn
  refuses it and no turn is reopened. One observation runs per session; edits
  meanwhile join the next one. The row is a Cowboy-origin prompt
  (`code_intelligence`, captioned "Code intelligence after an edit") carrying
  the same escaped JSON as the Claude row. Shell-made changes, subagents and
  delivery identity states are not covered. Workers without next-checkpoint
  controls refuse that mode before any effect; delivery then uses ordinary
  current-turn steering with the same turn target.

  Hawk's Controller runs this from `1b9114700` (transaction
  `1794139112893649931-1b911470086f`, `succeeded` / `committed`; Web
  `bc59fbd81`). In a new Codex 3.5.21 session, the first run showed the
  pipeline reaching steering but `sleep 8` ended the turn first; the second
  was refused as "next checkpoint is unavailable in this running worker",
  which led to the fallback. The third created a Go file, introduced an
  undefined name and ran `sleep 25`: the Controller logged
  `cowboy.code.post_edit_push` `nothing_to_deliver` for the clean file and
  `injected` after the edit. The row (`introduced: 1`,
  `undefined: thirdMissing`) entered the same turn before the shell call, and
  the model quoted it. Delivery is therefore bounded by the edit's turn:
  a turn that ends within the observation (a few seconds, longer for a cold
  file) receives nothing. The session was trashed.
- **Coverage.** Push triggers on any tool completion with a watcher-observed
  change set since the tool started (shell, formatters), not on
  `apply_patch`/Edit only.

  Implemented for shell commands with the existing protocol-42 observer, no
  Machine change. The Controller now reads the standing observer's pages
  (`code_observer::Read`), which it previously only started and closed. A
  per-session cursor is set to the observer head after a post-edit read (the
  observer starts on that read's diagnostic demand); a later completed shell
  command reads the changes since then. A Codex `execute` tool call triggers
  this from the Controller; Claude's Mod asks with
  `code.post_edit {changed: true}` after a main-agent foreground Bash. Gaps
  and resets move the cursor and fall back to the files the session already
  holds (their reads reload a changed mirror; identical results are
  suppressed). The protocol-42 observer turns any create, delete or rename
  (for example `sed -i`'s temporary file) into a coverage gap that drops the
  changes before it, so this fallback is the common shell path. Otherwise up
  to five changed files with an extension,
  outside `.git`, `node_modules`, `target`, `dist` and `.cache`, then go through
  the same read, comparison, ledger and delivery as an edit. A session whose
  observer never started (no earlier post-edit read, no Cowboy MCP grant, or
  an older Machine) reports nothing. Coverage is the observer's: event stream
  only, so absence never means unchanged.
- **Delivery identity.** Recipient (main agent; native subagents excluded
  initially), conversation/context identity, idempotent delivery ID and states
  `queued | appended | consumed | unknown`. Only evidenced model deliveries
  count against repetition limits. `context_cleared` drops pending deliveries;
  compaction and cold resume keep them as labelled historical observations.

  Implemented subset: a bounded Controller-memory ledger (20 per session, 256
  sessions; lost on restart) gives each delivery an ID, route (`adapter` for a
  `code.post_edit` result an adapter such as Claude's Mod appends,
  `codex_steering`), files and a digest of the shown JSON. States are
  `computed`, `steering`, `appended`, `refused`, `unknown` and `suppressed`;
  `consumed` is not observable yet. A result identical to the previous
  possibly-delivered one within 10 minutes is `suppressed` (`code.post_edit`
  returns `deliver: false, suppressed: "repeat"`); refused deliveries do not
  count. `code.summary()` lists the ledger as `deliveries` without repository
  text. Codex steering is bound to the edit's turn, so a cleared context
  refuses it; the Claude Mod's own appends are not reported back.
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
