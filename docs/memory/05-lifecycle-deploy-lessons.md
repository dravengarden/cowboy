# Lifecycle, deploy, and lessons

## `setup_memory` — wiring on `serve`

When `--memory-enabled` is set, `serve()` calls `setup_memory`, which:

1. inits the file+git store at `--memory-root` (`ensure_git_repo`);
2. **ensures one janitor session** — reuses a restored system session matching
   **`system && cwd == memory_root && provider == --memory-janitor-provider`**,
   else creates a fresh one (`new_session(origin = Api, system = true)`);
3. builds the initial keyword index (leniently — see below);
4. spawns the **reconcile loop** (drains the queue's mpsc channel → `run_janitor`)
   and a **12h tidy timer**.

Because the janitor is a normal supervised session marked `system`, it inherits
restart-recovery from the [Supervisor](../architecture/03-supervisor.md) and
**auto-approval** of its tool calls in the ACP handler — a system session has no
human to approve permission prompts, so cowboy approves them for it. It is also
hidden from the normal session list.

If memory setup fails for any reason, cowboy logs the error and runs **with
memory disabled** — a memory problem never takes the daemon down.

## `apply::resolve` — landing a decision

`apply::resolve(&store, &ops)` walks the parsed `ResolveOp`s:

- `Write` → write/overwrite the memory in its tier (dedup-updates reuse the
  existing name);
- `Archive` → move the file to `archive/`;
- `Move` → relocate between tiers.

It regenerates the affected `MEMORY.md` indexes and makes **one git commit** for
the batch, returning the commit hash (`None` when nothing actually changed —
e.g. a dedup that produced identical content). After a commit, cowboy rebuilds
the in-memory keyword index so later reconciles see the new state.

## Deploy

The service (`services/cowboy` on hawk) enables memory with three flags:

```
--memory-enabled
--memory-root /home/draven/.agents/memory
--memory-janitor-provider codex        # spares the Claude Max pool
```

`cowboy.service` also sets `CLAUDE_CODE_REMOTE_MEMORY_DIR` to the same root, so
the CC sessions cowboy spawns write their native auto-memory into the same store
the janitor curates. `git` must be on the unit PATH (the store commits shell out
to it).

### Deploying from inside a cowboy session

A subtle gotcha, because agents on this host often *are* cowboy sessions
(ancestry: agent → `claude-agent-acp` → `cowboy.service`). A normal
`nixos-rebuild switch` restarts cowboy, and — since the switch shares
`cowboy.service`'s cgroup — the restart SIGKILLs the switch mid-flight, leaving a
half-applied system. The fix is **build-as-draven, activate-detached-as-root**:
build the closure unprivileged, then activate it in a transient unit outside
cowboy's cgroup (`systemd-run … switch-to-configuration switch`), which survives
the restart; cowboy's auto-resume brings the turn back to verify.

### Retiring / restoring mnemosyne

The old `./services/mnemosyne` import is commented out (the module file is
kept). Memory now lives entirely in cowboy; the mnemosyne daemon, its two unix
sockets, and its MCP registrations are gone. The store **format is unchanged**,
so if the cowboy path ever regresses, uncommenting the import + rebuild brings
mnemosyned straight back on the same files.

## Lessons from the live cutover

Three bugs, each found only against the real store, each worth remembering:

1. **Empty-ops drop.** Codex read the reconcile prompt as "nothing to merge →
   `[]`" and silently discarded new candidates. Fix: make `write` the explicit
   default and warn that `[]` discards. The MCP janitor never hit this because
   the `memory_resolve` *tool* nudged it to act; a structured-text judge needs
   the obligation spelled out.
2. **Format + resilience.** The store mixes nested `metadata.type` and flat
   CC-native `type:`; a parser that only understood one shape errored on ~28 % of
   files, and because the index build was all-or-nothing, the *first* bad file
   blanked the whole index. Fix: parse both shapes, and build the index
   leniently (skip + warn).
3. **Wrong-provider janitor.** The session-reuse check matched `system && cwd`
   only, so a stale **claude-code** session at the old janitor cwd got grabbed as
   the "codex janitor"; its revived turn hung to the 180s timeout on every
   reconcile. Fix: match `provider` too, and put the janitor cwd at the store
   root (no pre-fold leftovers live there).

## Known limitation

The structured-text judge occasionally replies in prose (no ` ```json ` fence)
for a **large, dedup-update** candidate, even after the one retry — that proposal
is then left unwritten (logged). Typical candidates go through fine. If it
recurs often, harden the prompt or accept a non-fenced JSON array. It is a
robustness ceiling of asking a chat model for a machine-readable reply, not a
correctness bug — nothing wrong is ever written; a proposal is at worst dropped.
