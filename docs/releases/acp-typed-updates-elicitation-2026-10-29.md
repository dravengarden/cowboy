# Typed compaction, notices and elicitation — 2026-10-29

Status: Controller, Web and Hawk Machine deployed; OVH and Falcon not yet.

## Change

- `agent-client-protocol` 2.2 → 3.6 (schema 1.9.1 → 1.13.0).
- The ACP client advertises `session.compaction`, `session.notices` and form
  and URL `elicitation` (`src/acp.rs` `initialize_request`). codex-acp 2.x
  reserves the tool-call `contextCompaction` metadata for JetBrains AIR clients,
  so Cowboy had lost the signal for Codex compactions. Without elicitation,
  claude-agent-acp disables `AskUserQuestion` and codex-acp answers every
  `request_user_input` with nothing.
- Elicitations travel through the existing permission channel; see
  `design.md` "Minimal client capabilities". The transcript records only the
  outcome.

## Verification

| Gate | Result |
| --- | --- |
| `just check-compact` at `002e101c` | Passed |
| Elicitation browser conformance (`just cdp-browser-conformance … elicitation`) | 4 of 4 in Chrome 151 |
| `agent-worker-conformance`, Claude Code 3.20.18 (CLI 2.1.312, ACP 0.99.0), worker of `002e101c` | `initialize_and_session_new` and `stop_and_descendant_drain` passed |
| `agent-worker-conformance`, Codex 3.5.11 (CLI 0.164.0, ACP 2.3.0), same worker | Same two checks passed |

These harnesses never prompt a model. A live elicitation, compaction or notice
was not exercised before activation.

## Deployment

| Lane | Receipt |
| --- | --- |
| Web | `002e101c`, Hawk component activator, success |
| Controller | `002e101c`, Hawk component activator, success; `/healthz` ok |
| Hawk Machine | `cowboy-machine-release` `ifcdwqhd…`, `--maintenance`, `succeeded`/`committed`, generation `worker-4e08ffcc509b571bad02` |

Existing Hawk workers keep their generation until their session revives; new
launches use the new generation. Until a session runs this worker, its adapter
keeps the previous presentation.

The change touches `Cargo.toml` and `Cargo.lock`, so host-only Machine releases
refuse to build until the `cowboy-workers` pin advances to a revision that
contains it.
