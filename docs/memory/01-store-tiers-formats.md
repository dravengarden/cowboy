# Store, tiers, and frontmatter formats

The store is a **git repository of markdown files** at `~/.agents/memory`
(`--memory-root`, matching the `CLAUDE_CODE_REMOTE_MEMORY_DIR` cowboy sets for
the agents it spawns). One fact per file; every write is a git commit, so the
whole history of the machine's memory is recoverable.

## Layout

```
~/.agents/memory/
├── machine/                     # host-wide facts (services, conventions, hardware)
│   ├── MEMORY.md                # the tier index: one line per memory
│   └── <name>.md
├── projects/<cwd-slug>/memory/  # per-project tier, keyed by the session's cwd-slug
│   ├── MEMORY.md
│   └── <name>.md
└── archive/                     # soft-archived (forget never hard-deletes)
    └── <name>.md
```

`Tier` (in `store.rs`) is just the relative directory: `machine`, `archive`, or
`projects/<slug>/memory`. `store.rs` maps a `Tier` to its on-disk dir, reads and
writes `<name>.md`, and regenerates that tier's `MEMORY.md` after every change.

## Tiers, and how a proposal is routed

Two tiers hold live memories (`archive` is the graveyard):

- **machine** — facts true of the whole host: which services run where, host
  conventions, hardware quirks. Not tied to any one repo.
- **project** — facts scoped to one Columbus project, under
  `projects/<cwd-slug>/memory/`. A thing learned while working in project X
  stays out of the global store unless it genuinely belongs there.

`tier.rs` routes each proposal to a tier from the **cwd-slug** of the session
that proposed it. The slug is Claude Code's own keying: it derives a directory
name from the session's *current working directory* (e.g.
`/home/draven/columbus` → `-home-draven-columbus`), which is why opening a
cowboy session in a project's worktree makes that project's memory tier load.
The keying is by **cwd, not git root**, so a subdir project gets its own tier.

## The `Memory` record

```
Memory { name, description, mem_type, body }
```

- `name` — kebab-case slug; the `.md` stem and the join key for dedup.
- `description` — the one-line **recall hook**; this is what makes a memory
  findable, and what the `MEMORY.md` index and the keyword index rank on.
- `mem_type` — one of `user | feedback | project | reference` (`MemoryType`).
- `body` — the fact itself (for `feedback`/`project`, conventionally followed by
  `**Why:**` / `**How to apply:**` lines).

## Two frontmatter formats — both must parse

This is the subtle part. The store holds **two** frontmatter shapes side by
side, because two different writers produce them:

**Nested** (mnemosyne / the janitor write this):

```markdown
---
name: theia-project
description: theia OSINT project — stack + deploy decisions
metadata:
  node_type: memory
  type: project
  originSessionId: 87aa9052-…
---
body…
```

**Flat** (Claude Code's *native* auto-memory writes this, directly into the
store via `CLAUDE_CODE_REMOTE_MEMORY_DIR` — cowboy never sees these writes):

```markdown
---
name: cli-help-for-ai
description: dump everything in one --help
type: feedback
originSessionId: 2007f184-…
---
body…
```

`Memory::parse` walks the frontmatter line by line and captures `type` from
**either** a nested `metadata.type` **or** a flat top-level `type:`. Getting
this wrong is not cosmetic: a live store of 121 memories was ~72 % nested and
~28 % flat, and an early parser that only understood the nested shape errored on
every flat file — see [the lessons](05-lifecycle-deploy-lessons.md).

## The `MEMORY.md` index

Each tier carries a generated `MEMORY.md`: a bullet list of
`- [name](name.md) — description`. It is the human- and agent-facing table of
contents an agent reads *first* during recall (see
[recall](02-read-recall.md)). `store.rs` owns it — it is regenerated from the
tier's files on every write, never hand-edited, so it can never drift from the
actual set of files.
