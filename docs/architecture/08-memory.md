# Matrix memory boundary

Matrix owns long-term extraction, consolidation, retrieval, revision, forgetting
and checkpointing. It is a separate product and filesystem instance repository,
not a Cowboy database table or Controller endpoint. Native agents still own
their conversation history, compaction, authentication and execution.

Memory is available to every Agent on every Machine that has the `matrix`
Plugin active, and it is never required: see [the reliability
contract](../matrix-memory.md#reliability-contract). The standard Codex and
Claude signed adapters disable native automatic memory, recall bounded
historical context before a user turn, provide Matrix MCP tools, and durably
queue public observations for Matrix's background worker. They do not read or
rewrite native memory databases. Existing sessions retain their recorded Plugin
generation; publication cannot retrofit a running process. Grok, DeepSeek
Harness and the Claude and Codex DeepSeek variants have no such adapter, so the
ACP worker provides the same behavior for them and leaves their native memory
as it is.

A Machine with a local Matrix binding talks to it directly. Every other Machine
reaches the home Machine through its authenticated Cowboy connection, so memory
uses the route the Machine already has and needs no tunnel, proxy setting or
credential. The Controller derives the session's identity from its own record.
Explicit mappings on the home Machine join workspaces to one logical Matrix
project. The Matrix server independently fixes user/Provider/runtime and checks
grants.

`cowboy.memory-client` and the worker's `acp_memory` contain only bounded
delivery. Each Agent has its own Matrix credential and private provider scope,
and shares user, project and machine scopes. Isolated variants never read
standard Codex/Claude Matrix configuration or stores.

Required guidance belongs in AGENTS.md, documentation, tests and hooks;
reusable procedures belong in skills. Memory is fallible historical evidence
and cannot override current instructions or observed state. Failed recall gives
an unavailable marker, with no stale cache or native-memory fallback.
Completed observations remain in a private bounded outbox during outages.

See [configuration and acceptance](../matrix-memory.md) and the independent
[Matrix product](https://github.com/dravengarden/matrix). The `matrix-ovh`
instance owns its access configuration, memory journal and Git checkpoints.
