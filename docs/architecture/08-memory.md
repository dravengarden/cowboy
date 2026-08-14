# Agent-owned memory boundary

cowboy does not implement an agent-memory store. It launches each agent as the
human user with that runtime's native home. Codex uses the normal `CODEX_HOME`;
Grok uses `~/.grok` with its experimental native memory enabled for Cowboy
sessions. Both stores remain owned by their agent runtime.

This is a deliberate ownership boundary:

- Codex and Grok each own memory extraction, consolidation, relevance
  selection, storage, per-task controls, and rate-limit policy.
- Repository guidance that must always apply belongs in `AGENTS.md`, checked-in
  documentation, tests, or hooks.
- Reusable procedures belong in skills.
- Active work belongs in the current thread, Codex Goal mode, or the harness
  task graph.
- cowboy owns only session transport, persistence, process lifetime, and the
  client-facing control plane.

There are no cowboy memory CLI commands, HTTP endpoints, background janitor
sessions, reconcile loops, or scheduled tidy jobs. Generated memory under
`~/.codex` and `~/.grok/memory` is tool-owned state: cowboy neither reads nor
mutates it. Columbus may audit follower stores read-only for stale filesystem
references, but it does not bridge or rebuild them.

The old in-process mnemosyne port was removed because it duplicated Codex's
native capability and depended on a second agent session to judge the first
agent's generated notes. Its legacy store can remain offline for rollback or
manual knowledge promotion, but it is never loaded into a cowboy session.
