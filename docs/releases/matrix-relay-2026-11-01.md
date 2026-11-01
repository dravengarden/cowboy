# Matrix on every Machine and Agent, 2026-11-01

Delivers [the Machine relay](../matrix-memory.md#every-machine-and-every-agent):
Machine protocol 41, memory-client 0.2.1, `matrix` 1.0.5 (adds macOS) and Matrix
0.4.6 with the Grok, DeepSeek Harness and DeepSeek variant clients.
`memory.home_machines` is `["ovh"]`.

## Acceptance

Real sessions called `memory_search` and received recall:

| Runtime | Accepted |
|---|---|
| Hawk | Codex, Grok, DeepSeek Harness, Claude DeepSeek, Codex DeepSeek |
| Falcon | Codex, Grok, DeepSeek Harness |
| OVH | Claude, Codex (direct) |
| MacBook Air | Codex, Grok, DeepSeek Harness, Claude DeepSeek, Codex DeepSeek |

With Matrix stopped, running Claude DeepSeek, Codex and DeepSeek Harness sessions
kept answering, and new Codex, Grok and Claude sessions started and answered. Their
queued turns were delivered after Matrix returned. A Codex session started during
an outage reached `memory_search` after recovery, without a reload, once that
Machine's relay had learned discovery.

## Known limits

- A Machine whose relay has never completed MCP discovery cannot answer it during
  an outage. Any session completing startup while Matrix is up fills the cache.
- Grok and DeepSeek Harness cannot run with an OVH runtime and a remote execution
  target; this is existing Provider placement, not a memory limit.
- A retained session keeps its launch environment until it reloads.
- Another local process that takes the relay port while the Machine restarts could
  receive a retained session's token. Accepted: it needs a local account, and the
  token reaches only that session's memory.
