# ACP subagents and client extras — 2026-10-31

Status: Controller, Web and every Machine run `92b521d4` (worker
`worker-a592632327901448b247`, Machine protocol 40).

## Change

- ACP subagents (`docs/subagents.md`): the worker declares
  `clientCapabilities.subagents`, routes child sessions out of the parent
  timeline into one live, durable row per subagent, tags child permission and
  elicitation requests with the subagent, and stops one subagent on request
  (`cancel_subagent`, Cowboy MCP `sessions.cancel_subagent`, protocol-gated).
  A parent Stop also cancels every working, cancellable child, keeping "Stop
  stops everything" while claude-agent-acp spares children in this form.
- Client extras: Claude AskUserQuestion option previews, Codex
  auto-resolution countdown, URL elicitations closed by `elicitation/complete`,
  and the standard ACP tool `name` for tool presentation.
- Dormant until upstream changes: prompt suggestions (both adapters publish
  them only to JetBrains AIR clients) and Codex `mcpStartupAwaitTimeoutMs`
  (honoured only for ACP `mcpServers`; Cowboy injects Codex MCP through `-c`).

## Verification

| Gate | Result |
| --- | --- |
| `just check-compact` (subagents, then combined branch) | Passed |
| Browser conformance `elicitation`, `subagents`, `desktop-composer`, light and dark | Passed |
| `agent-worker-conformance`, new worker vs. installed claude-code 3.20.22 and codex 3.5.14 | `initialize_and_session_new` and `stop_and_descendant_drain` passed |

## Delivery

| Lane | Result |
| --- | --- |
| Controller, Hawk Machine, Web | Hawk component activators committed `92b521d4` |
| Falcon Machine | Host release through Falcon's activator, committed |
| macbook-air Machine | Rust 1.98.1 build and owned `--refresh`; hashes match; launcher unchanged |
| OVH Machine | Third attempt accepted: `/var/lib/columbus/ovh-machine-92b521d468ac-20261031T041636Z` |

The first two OVH attempts rolled back by their own timers:

- A Claude · Mink (named account) session's live worker could not be replaced
  by a cutover (`starting transient worker unit`): a named-account launch is
  refused while the previous worker still holds the binding. It had not been
  hibernated beforehand. It was reloaded after the accepted maintenance and
  resumed its native session on the new worker.
- A Codex session could not resume on either generation: the OpenAI account's
  authorization had expired ("Sign in to Codex again"), so Codex fell back to
  the package's API-key auth and failed. It needs a fresh Codex sign-in; this
  is unrelated to the release.
