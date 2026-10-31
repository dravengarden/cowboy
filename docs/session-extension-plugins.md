# Session extension Plugins: memory backends and call profiles

Status: design, 2026-10-15. Implementation is staged below; each stage states
what it ships and what it leaves unchanged.

## Problem

Two session capabilities are integrations compiled into core and duplicated in
Provider Plugins, contrary to CR-0 and CR-1:

- **Matrix memory.** Core stores `matrix.tools` and `matrix.recall` switches
  (`src/server/agent_tools.rs`, `web/src/agentTools.ts`); `components/memory-client`
  is a Matrix client; `plugins/codex` and `plugins/claude-code` each bridge
  Matrix, and Claude hard-codes the seven Matrix MCP tool names. Enrollment is
  a host file, `~/.config/matrix/<provider>.json`, outside the Plugin
  lifecycle. The Controller switches are stored but no worker reads them, so
  turning memory tools or recall off has no effect.
- **Review calls.** The managed-call purposes (`review`, `design_review`,
  `analysis`) are a closed core enum, and the agent-facing guidance that tells
  an agent when and how to ask for a review is a string copied into each
  Provider Plugin (`AGENT_CALLS_PROMPT`).

## Boundary

The managed-call mechanism stays core: grants, Machine protocol, snapshots, the
read-only network-free sandbox, split placement and the per-Provider
`read_only_v1` profile are execution security, which CR-0 reserves to the
generic layer. What moves to Plugins is what is integration-specific:

| Concern | Owner after this change |
|---|---|
| Memory protocol, tool names, recall/observe capability | `memory_backend` Plugin (data-only contract) |
| Memory endpoint, token, project mapping | Machine-private binding file, never signed package data |
| Memory driver code | `cowboy.memory-client` component, one closed driver per protocol |
| Per-agent and per-session memory switches | Controller (generic, no backend name) |
| Call purposes, agent-facing review guidance | `call_profile` Plugin (data-only contract) |
| Call grants, snapshot, sandbox, placement | Core (unchanged) |

## `memory_backend` Plugin kind

Release schema 5, payload schema 1, Plugin SDK 1.12+. Data-only like
`telemetry_backend`: no executable artifacts, URLs or credentials. Catalog
readers before session extensions accept release schemas up to 4, so they skip
these envelopes instead of failing to decode an unknown kind.

```json
{
  "schema_version": 1,
  "id": "matrix",
  "version": "1.0.0",
  "display_name": "Matrix",
  "description": "Shared, evidence-backed memory across Codex and Claude sessions.",
  "supported_platforms": [{"os": "linux", "architecture": "x86_64"}],
  "protocol": "matrix_http_v1",
  "tools": ["memory_search", "memory_get", "memory_put", "memory_forget",
            "memory_read", "memory_execute", "memory_receipt"],
  "recall": true,
  "observe": true
}
```

`protocol` is a closed enum implemented by `cowboy.memory-client`; adding a
protocol is a component release, adding a backend that speaks an existing
protocol is only a Plugin. Tool names are bounded (at most 16, identifier
syntax) and become the exact MCP allow-list a Provider may expose.

The binding stays private to the Machine user that runs the Provider:
`$XDG_CONFIG_HOME/cowboy/memory/<plugin-id>/<provider>.json` (mode 0600, owner
only), with the existing Matrix fields (`endpoint`, `token`, `state_dir`,
`projects`). For compatibility the `matrix` Plugin also accepts
`~/.config/matrix/<provider>.json`; this fallback is deleted once OVH has a
binding at the new path.

## `call_profile` Plugin kind

Release schema 5, payload schema 1, Plugin SDK 1.12+, data-only.

```json
{
  "schema_version": 1,
  "id": "review",
  "version": "1.0.0",
  "display_name": "Review",
  "purposes": [
    {"id": "review", "label": "Code review"},
    {"id": "design_review", "label": "Design review"},
    {"id": "analysis", "label": "Analysis"}
  ],
  "guidance": "<agent-facing text, at most 4 KiB>"
}
```

Payload schema 1 may only label the purposes core managed calls accept
(`review`, `design_review`, `analysis`); a profile-defined purpose needs a
later payload schema together with core request validation, so it is refused
today rather than advertised and then rejected. Guidance replaces each
Provider's copy of `AGENT_CALLS_PROMPT`; `{{peer}}` and `{{peer_command}}` are
the only placeholders.

## Delivery to a session

The Controller resolves, per session, the enabled memory backend (an installed
`memory_backend` release on the Provider's runtime Machine, or none) with the
effective tools/recall switches, and the enabled call profiles. It sends the
result with the worker launch. The Machine writes it as a private session
descriptor and exposes its path to the Provider launcher as
`COWBOY_SESSION_EXTENSIONS`:

```json
{
  "schema": 1,
  "memory": {
    "plugin": {"id": "matrix", "version": "1.0.0", "digest": "sha256:…"},
    "protocol": "matrix_http_v1",
    "tools": ["memory_search", "…"],
    "enable_tools": true,
    "enable_recall": true,
    "observe": true
  },
  "calls": {
    "purposes": ["review", "design_review", "analysis"],
    "guidance": "…"
  }
}
```

On a Machine that runs the memory relay, `memory.relay` adds the session's
loopback `endpoint`, its `token` and its private outbox `state_dir`. A launcher
without a local binding uses it; see
[Matrix Provider integration](matrix-memory.md#every-machine-and-every-agent).

Provider launchers read only this descriptor and the private binding it names.
No descriptor (a Machine that predates session extensions) keeps the legacy
host enrollment with both switches on and the release's built-in call
guidance. A Machine that cannot resolve or write the descriptor sets
`COWBOY_SESSION_EXTENSIONS=disabled`, so the session runs without memory
instead of falling back to an enrollment that ignores its switches. A
descriptor that later disappears or is unsafe or malformed is treated the
same way by the Provider. The Machine carries at most eight call profiles and
refuses to publish a descriptor larger than the Providers' 64 KiB read limit. Both
switches off means no memory at all, including observation. A backend that
recalls without observing still answers context. Codex exposes only the
backend's declared tools through `enabled_tools`; Claude allows exactly
their `mcp__<plugin>__<tool>` names.

The Tools documents keep schema 1 and the `matrix` wire and storage key, so
deployed Web clients and stored settings keep working; the Controller also
accepts `memory`, and the Web client renames the key at its boundary.

## Stages

1. SDK: `memory_backend` and `call_profile` kinds, contracts, validation,
   build/sign/verify; first-party `plugins/matrix` and `plugins/review`
   packages; `just plugin-check`.
2. Controller: generic `memory` switches replacing `matrix` (stored settings
   with the old key keep loading), resolution of installed extensions,
   session-extension payload on worker launch; Web panel labels from the
   installed Plugin.
3. Machine and worker: accept the new kinds, write the descriptor, set
   `COWBOY_SESSION_EXTENSIONS`. Requires a Machine maintenance window.
4. Provider Plugins: `memory-client` drivers keyed by protocol, Codex and
   Claude bridges and the Claude MCP allow-list from the descriptor, call
   guidance from the descriptor; the switches take effect.
5. Publication and rollout: sign and publish `matrix` and `review`, install on
   OVH/Hawk, move the OVH binding to the new path, remove the legacy fallback.

## Deletion triggers

- The `~/.config/matrix` fallback: after OVH and Hawk have bindings at the new
  path and one release has run without reading it.
- Built-in purposes and guidance: after every active Provider generation reads
  the descriptor and the `review` Plugin is installed wherever calls run.
