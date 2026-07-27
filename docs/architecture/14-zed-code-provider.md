# Isolated Zed Code Provider

Cowboy Code may use Zed's headless project, worktree, buffer, and LSP logic,
but it must not link that implementation into the MIT-licensed Cowboy daemon.
The Zed crates and wire schema used by remote development are
GPL-3.0-or-later. They therefore live in a separately built and distributed
`cowboy-zed-adapter` process.

## Version boundary

The adapter pins one Zed source revision and one matching remote-server build.
The initial compatibility target is:

- remote server version: `1.13.0`;
- Zed revision: `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`.

An upgrade changes both values together and must pass the adapter protocol
fixtures before deployment. Cowboy never discovers or executes the mutable
server under `~/.zed_server`; Nix supplies the exact adapter/server pair.

## Process and state isolation

There is one adapter instance for Cowboy, shared by Code sessions. It has its
own runtime, data, cache, logs, extension store, and trust database:

```text
/run/user/1000/cowboy-zed/
/var/lib/cowboy-zed/
  data/
  cache/
  extensions/
  trust/
```

It never reads or writes Hawk Zed's mutable state. A read-only extension source
may be populated from the same declared extension set, but installation state
is private. Project `.zed/settings.json` and `.editorconfig` remain shared
because they belong to the worktree. UI settings are ignored.

## Trust

Opening a worktree does not execute project-provided language-server, task, MCP,
or formatter commands until Cowboy records an explicit trust grant for that
canonical path. Restricted worktrees still provide files, Git state, and
client-side syntax highlighting. The stable API reports restricted capability
instead of silently treating absent LSP results as success.

## Stable socket contract

Cowboy talks to the adapter over a private Unix socket. The adapter owns Zed
`Envelope`, project/worktree IDs, scan IDs, buffer IDs, language-server IDs,
reconnect, and replay. Cowboy sees only versioned product messages:

- idempotently ensure a worktree for manifest/capability discovery without
  acquiring a UI lease;
- open/close a canonical worktree lease;
- subscribe to monotonic worktree deltas;
- open/close a read-only buffer lease;
- request diagnostics, semantic tokens, inlay hints, hover, definitions,
  references, and document symbols;
- inspect capability, restricted, warming, ready, and failed states.

Requests include a worktree revision and buffer revision. Stale responses are
discarded at the adapter boundary. Reconnect rebuilds leases and emits a fresh
snapshot before deltas resume.

## Failure behavior

The existing `LocalCodeProvider` remains the file/Git fallback. Adapter failure
must not blank Code, block Agent traffic, or restart Cowboy. It disables only
language intelligence and provider-driven deltas, exposes the reason in
capabilities, and retries with bounded backoff. A low-frequency manifest
reconciliation remains active even after delta streaming is healthy.

The first production slice implements the idempotent ensure and leased
open/close operations against the pinned Zed server. Cowboy's manifest reports
`provider: "zed"` and `state: "ready"` only after the ensure response succeeds;
individual LSP capabilities remain false until their stable adapter messages
are implemented.
