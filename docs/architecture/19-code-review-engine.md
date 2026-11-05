# Code Review Engine: capabilities first, implementations hidden

**Status: design proposal (2026-11-05), not implemented.** It reframes the
current "Zed provider" as one internal part of Cowboy's **Code Review** product.
Users, agents and public APIs see Code Review capabilities only. Each capability
is implemented by whatever internal component serves it best; Zed is one such
component today and has no privileged position.

Related: [mobile Code Review](13-code-review.md),
[isolated Zed component](14-zed-code-provider.md),
[agent code intelligence](20-agent-code-intelligence.md).

## Why

Today the boundary is drawn around an implementation:

- Product surfaces name it: "Zed default directory", "Zed defaults",
  "Find a Zed directory", folder-defaults help text
  (`web/src/folderDefaults.ts`, `ReviewDirectorySetting.tsx`,
  `ReviewDirectoryPicker.tsx`, `mobile/review/ReviewApp.tsx`,
  `FolderDefaultsEditor.tsx`), the MCP preference scope
  `preferences.get("zed")` (`src/agent_api.rs`), the Code manifest's
  `provider: "zed"`, and the installable Plugin id `zed`.
- The Controller routes every language request to the literal adapter name
  `"zed"` (`code_reads/session.rs`, `code_buffers/remote.rs`,
  `zed_session.rs`), and response types are named after it
  (`ZedAdapterResponse`).
- The `cowboy.code-intelligence` contract lists operations, but a Machine is
  assumed to have exactly one implementation of all of them.

That makes "should this feature use Zed?" an architectural default instead of
an engineering decision, and leaks a component choice into user vocabulary.

## Target model

```text
 Users / Web / Desktop / iOS          Agents (cowboy MCP, Mods, hooks)
            │  Code Review product API      │  cowboy.code.* / review.*
            └──────────────┬────────────────┘
                 Code Review Engine (Controller)
                 • capability API, freshness states, scope/authority
                 • capability router: policy table per capability
                           │
          Machine: capability implementations (private, swappable)
   ┌──────────────┬───────────────┬───────────────┬─────────────────┐
   core worktree   language engine  structure       review analysis
   (fs, Git,       (LSP lifecycle;  (syntax trees,  (Calls: AI review,
   search; Rust)   Zed today)       tags; candidate) annotations)
```

### Vocabulary

| Visible name | Meaning |
| --- | --- |
| Code Review | The product: worktree, changes, files, language intelligence, review findings |
| capability | A versioned function with a stable product schema, e.g. `changes@1`, `diagnostics@1`, `references@1` |
| implementation | Internal component that serves capabilities; never shown to users or agents |
| state | Per capability and scope: `ready`, `warming`, `settling`, `restricted`, `unavailable`, `failed` |

Results may carry an internal `implementation` field for telemetry and
debugging on operator-only surfaces; product and agent responses expose the
capability, `state` and, where meaningful, `precision`
(`exact | heuristic`), never an implementation name.

### Capability router

The Engine owns a versioned **policy table**: for each capability, an ordered
list of implementations with fallback conditions. Selection is deterministic
per (Machine, worktree, language, trust state), not an auction. Example:

| Capability | Preferred | Fallback | Precision of fallback |
| --- | --- | --- | --- |
| `outline@1` | language engine (LSP symbols) | structure (syntax tags) | heuristic |
| `references@1` | language engine | text search with symbol filter | heuristic |
| `diagnostics@1` | language engine | none (state `unavailable`) | n/a |

A fallback answer is always labeled `heuristic`; it never masquerades as an
LSP answer. Restricted (untrusted) worktrees route only to implementations that
execute no project-provided code.

### Implementation contract

The existing `cowboy.code-intelligence` contract becomes an implementation
declaration rather than "the provider":

- each installed implementation declares the capabilities and capability
  versions it serves, per language where relevant, plus its trust requirement
  (`executes_project_code: true|false`);
- several implementations may be installed on one Machine; the Engine routes;
- the private socket protocol per implementation is unchanged, so the GPL
  isolation of the Zed component stays exactly as in
  [14-zed-code-provider.md](14-zed-code-provider.md).

## Capability inventory and implementation choice

Every capability is evaluated on the same rubric: first-paint latency,
precision, warm-up cost and memory, whether it executes project code (trust),
language coverage, fit for runtime-only Machines (OVH: static binaries, no
toolchains), license boundary, and maintenance/pin churn.

| Capability | Implemented by today | Assessment |
| --- | --- | --- |
| Manifest, directory pages, file pages, content-addressed cache | core worktree (`CodeProvider`, `cowboy-code-adapter`) | Keep. Fast, no project code, static binary. |
| Filename search | core | Keep. |
| Content search | not a product capability yet | Add on core with the ripgrep library (`grep` crates): no warm-up, works restricted. |
| Changes, commit list, commit/diff documents | core (Git) | Keep. Consider structural (syntax-aware) diff later as an additional `diff@2` from the structure implementation. |
| Syntax highlighting | browser CodeMirror/Lezer + snapshot of Zed first-line matchers | Keep client-side for zero round trips. Replace the Zed matcher snapshot dependency with a Cowboy-owned language-detection table. |
| Outline / document symbols | language engine (Zed) | Add structure fallback (tree-sitter tag queries) for first paint, restricted worktrees and languages without a server; LSP refines when ready. |
| Hover, definition, declaration, type definition, implementation, references | language engine (Zed) | Keep Zed for now: it already manages server installation, project settings and multi-server languages under the trust gate. Re-evaluate per language against a direct LSP host or SCIP indexes when a concrete gap appears. |
| Diagnostics (per file, project summary, post-edit delta) | language engine, per open buffer only | Extend in the language engine (project summary, version-matched pull). Project lint/typecheck commands remain a separate, explicit Review check, not an implicit LSP substitute. |
| Semantic tokens, inlay hints | language engine | Keep; optional enrichment only. |
| Workspace symbol search | none | Language engine when ready; structure tags fallback. |
| AI review findings, anchored annotations | Calls (`review.start`) | Part of Code Review: findings render as anchored annotations on the same diff and file documents. |
| Agent code intelligence | none | Consumer of the same capabilities, see [20](20-agent-code-intelligence.md). |

Adding a capability requires a short entry in this table naming the chosen
implementation and why the alternatives lost; Zed is never the default answer.

## Removing the implementation name from the product

| Surface | Change | Compatibility |
| --- | --- | --- |
| Web/Desktop/iOS strings | "Zed default directory" → "Review directory"; "Zed defaults" → "Review defaults"; folder-defaults help text | none needed |
| MCP preferences | scope `"review"`; `"zed"` accepted as a read/write alias, omitted from help | alias retained until no session SDK revision uses it |
| Preference storage | stored key unchanged (internal) or migrated with a dual reader first | stage reader before writer, per existing release practice |
| Code manifest | `provider: "zed"` → per-capability `state`/`precision` | Web reads both during one release |
| Controller internals | route by capability through the router; `zed_session.rs`, `ZedAdapterResponse` become language-engine names | internal only |
| Plugin identity | user-facing display name "Code Review language engine"; Plugin id `zed` stays an internal identifier until a planned identity migration | Plugin ids are signed release identities; renaming requires the explicit identity retirement path |
| Docs | product docs say Code Review; component docs keep Zed specifics | n/a |

## Phasing

| Phase | Scope |
| --- | --- |
| R0 | Remove user-visible Zed naming (strings, MCP `review` scope with alias, manifest state fields). No behavior change. |
| R1 | Capability router and policy table in the Controller; route existing Zed operations through it; implementation declarations in the contract. |
| R2 | Content search on core; structure implementation (tree-sitter tags) for outline fallback and restricted worktrees. |
| R3 | Language-engine extensions (project diagnostics summary, workspace symbols, version-matched diagnostics) feeding Review and agents (doc 20). |
| R4 | Per-language re-evaluation of the language engine against alternatives, driven by measured gaps. |

Each phase keeps the Web contract stable and adds conformance at the Engine
boundary, so implementations can be swapped without client releases.

## Open questions

1. Should the structure implementation run in the Controller, on each Machine,
   or in the browser (WASM), given OVH is runtime-only?
2. Is a Plugin identity migration from `zed` worth its release cost, or is a
   display name plus internal id sufficient indefinitely?
3. Which capability versions should agents see first: the full list, or only
   `diagnostics`, `references`, `outline` and `symbols`?
