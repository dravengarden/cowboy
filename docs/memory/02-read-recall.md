# Reading: recall and the index

Reads never touch cowboy. There is no `memory_query` tool, no endpoint, no
daemon round-trip — an agent recalls memory by grepping plain files, exactly the
way it already reads any other part of the repo. This is the "read = cost, so
remove the constraint" half of the [asymmetry](00-overview.md).

## The read path is a skill, not a tool

A machine-level `memory` skill (fanned out to `~/.agents/skills`,
`~/.claude/skills`, `~/.codex/skills`) teaches every agent the recipes:

```bash
# 1. Start from the indexes — the cheap overview
cat ~/.agents/memory/machine/MEMORY.md
ls  ~/.agents/memory/projects/                       # find your project's slug
cat ~/.agents/memory/projects/<slug>/memory/MEMORY.md

# 2. Read a specific memory by name
cat ~/.agents/memory/machine/<name>.md

# 3. Full-text across every tier
rg -i 'keyword' ~/.agents/memory/

# 4. Scan just the recall hooks
rg -n '^description:' ~/.agents/memory/
```

The `description:` frontmatter line is the recall hook; grepping descriptions
first is how an agent checks "do we already know this?" before recording.

A skill is the right shape here (not an MCP tool) precisely because the agent
*is* the operator — there is no auth boundary, no multi-tenant surface, nothing
an external protocol buys you. The recipes are cheaper than a resident tool
definition that would cost context on every turn whether or not memory is used.

## The keyword index (for the machine, not the agent)

`index.rs` builds an in-process **keyword index** over the store. Note the
audience: the index is not what agents recall through — they use `rg`. The index
exists so **cowboy's own janitor** can dedup a proposal against everything
already stored, cheaply, with no model call.

- `Index::from_store(&store)` walks every active tier and builds weighted token
  sets: tokens from a memory's `name` (weight 3), `description` (2), and `body`
  (1).
- `query(q, limit)` scores the query's tokens against every entry and returns
  ranked `Hit { name, description, tier, score, memory }`; `limit = 0` means no
  cap.
- `best_duplicate(&memory)` returns the single strongest overlap — the one the
  janitor shows the judge as "closest existing".

### Built leniently, on purpose

`from_store` uses `store.list_lenient`, which **skips** a file it cannot parse
(logging a warning) instead of aborting. This matters: an all-or-nothing build
once hit a single unreadable file and fell back to an **empty** index — which
silently cripples dedup, because the judge then thinks all 121 existing memories
are novel. One malformed memory must never blank recall for the rest.

## Why keyword, and not embeddings (yet)

The index ranker is deliberately a keyword scorer, not a vector store. The real
semantic engine is **the model plus the curated `description` index** — the
judge reads the candidate and the top keyword hits and decides. Embeddings / kNN
are only worth adding as an *internal* pre-filter at a scale where the index is
too large to hand the model whole; that is a swap behind `Index` with **no
change to any agent-facing interface**. Until then, keyword recall over a few
hundred well-described memories is more than enough, and it costs nothing to
build or query.
