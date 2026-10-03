# Matrix memory boundary

Matrix owns long-term extraction, consolidation, retrieval, revision, forgetting
and checkpointing. It is a separate product and filesystem instance repository,
not a Cowboy database table or Controller endpoint. Native agents still own
their conversation history, compaction, authentication and execution.

The standard Codex and Claude Plugins can opt in through a private runtime-host
configuration. Their exact signed adapters disable native automatic memory,
recall bounded historical context before a user turn, provide Matrix MCP tools,
and durably queue public observations for Matrix's background worker. They do
not read or rewrite native memory databases. Existing sessions retain their
recorded Plugin generation; publication cannot retrofit a running process.

For OVH runtime with Hawk/Falcon execution, Matrix requests and its outbox stay
on OVH. The execution descriptor supplies executor and workspace identity.
Explicit host mappings join those registrations to one logical Matrix project.
The Matrix server independently fixes user/Provider/runtime and checks grants.

`cowboy.memory-client` contains only bounded delivery. Codex and Claude own
their native protocol integration. Grok is deferred and retains native memory.
DeepSeek variants stay isolated and never read standard Codex/Claude Matrix
configuration, credentials or stores.

Required guidance belongs in AGENTS.md, documentation, tests and hooks;
reusable procedures belong in skills. Memory is fallible historical evidence
and cannot override current instructions or observed state. Failed recall gives
an unavailable marker, with no stale cache or native-memory fallback.
Completed observations remain in a private bounded outbox during outages.

See [configuration and acceptance](../matrix-memory.md) and the independent
[Matrix product](https://github.com/dravengarden/matrix). The `matrix-ovh`
instance owns its access configuration, memory journal and Git checkpoints.
