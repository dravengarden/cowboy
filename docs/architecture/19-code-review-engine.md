# Code Review Engine: capabilities first, implementations hidden

**Status: design proposal r3 (2026-11-05), partially implemented.** r3 folds in
two static design reviews (Fable 5.1, then Codex Astra Max). It reframes the
current "Zed provider" as one internal part of Cowboy's **Code Review**
product. Users, agents and public APIs see Code Review capabilities only. Each
capability is implemented by whatever internal component serves it best; Zed
is one such component today and has no privileged position.

Related: [mobile Code Review](13-code-review.md),
[isolated Zed component](14-zed-code-provider.md),
[agent code intelligence](20-agent-code-intelligence.md).

## Requirements

1. **Code Review is the only product concept.** Zed is never the default
   answer; every capability is evaluated on its merits.
2. **Deep integration with the agents Cowboy runs**, both directions
   (see [20](20-agent-code-intelligence.md)).
3. **No regression:** Code-pane latency and availability, GPL process
   isolation, trust gating, owner/authority scopes for code reads, native
   buffer ownership, agent turn availability (Code Review failure never blocks
   or fails a turn) and runtime-only Machines (OVH: static artifacts only).

## Why

The boundary is drawn around an implementation today: product strings name
Zed, the Controller routes every language request to the literal adapter
`"zed"` (`code_reads/session.rs`, `code_buffers/remote.rs`, `zed_session.rs`),
response types are named after it (`ZedAdapterResponse`), the Code manifest
carries `provider`, and `cowboy.code-intelligence` assumes one implementation
of every operation per Machine. That makes "should this use Zed?" an
architectural default instead of an engineering decision.

## Target model

```text
 Users / Web / Desktop / iOS          Agents (cowboy MCP, Mods, hooks)
            │  Code Review product API      │  cowboy.code.*
            └──────────────┬────────────────┘
       Controller: Code Review Engine (policy, trust, authority, projection)
                           │  standing observation continuation
       Execution Machine: Code Observer + capability implementations
   ┌──────────────┬────────────────┬──────────────┬──────────────────┐
   core worktree   language engine   structure      review analysis
   (fs, Git,       (LSP lifecycle;   (tree-sitter   (Calls: AI review,
   search; Rust)   Zed today, GPL    tags; Rust,    annotations)
                   process)          in core)
```

The **Controller** owns the policy table, the trust record, read/acquisition
authority and every product projection. The **execution Machine** owns
observation (watcher, executor receipts, cached analysis) and the
implementations, beside the files. Diagnostic text stays on the Machine until
an authorized read or delivery.

### Vocabulary

| Visible name | Meaning |
| --- | --- |
| Code Review | The product: worktree, changes, files, language intelligence, review findings |
| capability | A versioned function with a stable product schema, e.g. `changes@1`, `diagnostics@1` |
| implementation | Internal component that serves capabilities; never shown to users or agents |
| state | `ready`, `warming`, `settling`, `stale`, `restricted`, `saturated`, `unavailable`, `failed` |
| precision | `exact` or `heuristic` (a fallback answer is always `heuristic`) |
| coverage | What was observed: participating servers and their readiness, analysis frontier, observed scope, truncation |

Product and agent responses never name an implementation. An internal
`implementation` field exists only on operator telemetry.

## Trust (prerequisite for routing)

Today the Controller sends `"trusted": true` for every Session worktree
(`components/server/src/server/code_reads/session.rs`,
`components/server/src/server/zed_session.rs`), although
[14](14-zed-code-provider.md) requires an explicit grant. r3 defines the grant:

- A **core-owned, revocable trust record** keyed by (Machine, canonical root
  identity, implementation generation). Trust is **granted by a person**, never
  derived from execution or from an agent request.
- The grant is a **"Project language servers"** default in the existing
  folder/session defaults (inherited like `review_directory`), recorded per
  root on first use, plus a one-tap grant offered in Review.
- Migration keeps today's behaviour without silent trust: the top-level
  default is migrated to **on for session worktrees**, explicitly visible and
  switchable off. Browse-only contexts (`workspace::`, `worktree::`, linked
  sessions read without executing) default to **restricted**.
- **Revocation** fences new work, restricts the original backend, and observes
  quiescence of its project-code processes. The adapter today only promotes
  Restricted → trusted (`plugins/zed/adapter/src/main.rs`, `ensure_worktree`);
  revocation needs a new private operation.
- "Restricted" must be enforced, not just routed: the core Git reader starts no
  repository-configured program (fsmonitor, hooks, signature verification,
  external diff/textconv, clean/smudge/process filters;
  `src/code_review/git.rs`, implemented).

## Capability router

A versioned, deterministic policy table: per capability an ordered
implementation list with fallback conditions, keyed by
(Machine **platform**, worktree, language, trust state, saturation). Rules:

1. A fallback answer is labeled `heuristic` and never masquerades as exact.
2. Restricted worktrees route only to implementations declaring
   `executes_project_code: false`.
3. **Routing selects only new observations.** Handles, buffer owners,
   baselines, pending observations and navigation groups are bound to the
   immutable implementation, process and policy revision that created them;
   lifecycle operations (read, release, query) always return to that owner and
   are never re-routed or retried through a fallback.
4. A platform without an implementation reports `unavailable`, never
   `warming` (the language engine is Linux x86_64 only,
   `components/code-intelligence/contract.json`).

The router is generalized only after a second concrete implementation exists
(structure tags); until then it is a table with one row per capability.

### Implementation declarations

`cowboy.code-intelligence` becomes a declaration of capabilities (per
language), capability versions and `executes_project_code`. Several
implementations may be installed per Machine. The Zed private socket protocol
and GPL isolation are unchanged; new evidence (analysis frontier, revocation)
is a **versioned extension** of that protocol, probed on the exact retained
generation before it is advertised.

Schema changes are staged reader-before-writer: `WorktreeManifest.provider` is
required today (`src/code_review.rs`) and `CodeIntelligenceContract` rejects
unknown fields (`components/plugin-sdk/src/lib.rs`). New fields arrive first as
optional fields every retained reader accepts; removal happens only after no
retained Controller, Machine, Plugin generation or cached client needs them.

## Capability inventory and implementation choice

Rubric: first-paint latency, precision, warm-up cost and memory, executes
project code, language coverage, runtime-only Machine fit, license boundary,
maintenance/pin churn.

| Capability | Today | Choice and why |
| --- | --- | --- |
| Manifest, tree, file pages, cache, filename search | core | Keep: fast, no project code, static binary. |
| Content search (`search@1`) | none | Core, ripgrep library crates; bounded bytes/time/results; works restricted. First user-visible slice. |
| Changes, commits, diffs | core (Git) | Keep, with the hardened Git reader. Structural diff later as `diff@2` from structure. |
| Syntax highlighting | browser Lezer + Zed matcher snapshot | Keep client-side; replace the snapshot with a Cowboy-owned detection table shared with structure. |
| Outline, document symbols | language engine | Add structure fallback: tree-sitter grammars and tag queries **compiled into the execution-Machine core** (MIT, static, no project code, macOS too). Browser parsing stays presentation-only. |
| Workspace symbols | none | Structure tags first (bounded per-worktree index); language engine refines when trusted and warm. |
| Hover, definition, declaration, type definition, implementation, references | language engine | Keep Zed (server lifecycle, project settings, multi-server). Re-evaluate per language against a direct LSP host or content-matched SCIP only on measured gaps. |
| Diagnostics | language engine, per open buffer | Extend per [20](20-agent-code-intelligence.md): analysis-frontier evidence, observed-scope summaries. |
| Project checks (`check@1`) | agents run them in Bash | Ingest structured results (`cargo --message-format=json`, `tsc`, `go vet -json`, SARIF) as scoped evidence for their captured inputs. Running a check is an explicit, trust-gated effect in the session's execution lane, never a read and never on OVH. |
| Semantic tokens, inlay hints | language engine | Keep; enrichment only. |
| AI review findings | Calls `review.start` | Render as annotations anchored to the call's `input_revision`, file identity and diff side; stale or unmapped otherwise. |

Every future capability adds a row naming the chosen implementation and why
alternatives lost.

## Resource budgets

One headless Zed per Plugin generation per Machine serves every worktree, and
each worktree starts its own language servers. Before any automatic warming:

- per-Machine budgets for warm worktrees and language-server process trees
  (count, RSS, CPU), with interactive Code-pane traffic strictly first;
- automatic warming only for sessions that enabled agent intelligence, with
  idle shutdown of agent-only worktrees;
- `saturated` state when budgets are exhausted: intelligence degrades,
  execution is never affected;
- independent byte/time/concurrency bounds for search and parsing from day one.

Buffer-mirror accounting is not an RSS bound
(`docs/plugin-owned-buffer-reads.md`).

## Removing the implementation name from the product

| Surface | Change | Status |
| --- | --- | --- |
| Web strings | "Review directory", "Review defaults", "Code intelligence" chip, outline error text | implemented |
| Agent/SDK wording | "Review directory defaults"; scope id unchanged | implemented |
| MCP preference scope | canonical `code`; `zed` accepted as an undocumented alias. Both spellings normalize to one fingerprint representation compatible with already retained `zed` fingerprints and the `zed:{operation}` dedupe key (`src/core/review_defaults.rs`). Storage stays in folder/mobile-review state. | planned |
| Code manifest | add per-capability `state`/`precision` as optional fields; keep `provider` until no retained reader needs it | planned |
| Controller internals | route by capability; rename `zed_session.rs`, `ZedAdapterResponse` | planned |
| Plugin identity | signed id `zed` retained; product surfaces (`machines.read`, Extensions) show a projection named "Code Review language engine". Migration only with a separate justification; retained releases must stay resolvable. GPL attribution stays on the license surface. | decided |
| macOS installer component labels | operator surface; follow when that app next ships | deferred |

## Phasing

| Phase | Scope |
| --- | --- |
| R0a | User-visible strings; hardened Git reader (done) |
| R0b | `code` scope with fingerprint-compatible alias; optional manifest state fields (readers first) |
| R1 | Trust record, grant UI, revocation; non-projecting agent resolver (see 20); platform axis |
| R2 | Content search on core; tree-sitter structure in Machine core; router generalized with provenance binding |
| P0–P3 | Agent code intelligence per [20](20-agent-code-intelligence.md) |
| R3 | `check@1` ingestion and explicit runs (parallel with P0–P2) |
| R4 | Per-language re-evaluation of the language engine, driven by measured gaps |

## Decisions taken in r3 (owner may revise)

1. Trust is person-granted; session worktrees migrate to an explicit, visible
   "on" default; browse-only contexts are restricted.
2. The first user-visible slice is content search; agent diagnostics follow
   their safety gates.
3. The Plugin id `zed` stays internal; product projections hide it.
4. The preference scope is `code`.
5. Automatic refresh never counts as a save (no flycheck per agent edit).
