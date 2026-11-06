# Isolated Zed Code Provider

Cowboy Code may use Zed's headless project, worktree, buffer, and LSP logic,
but it must not link that implementation into the MIT-licensed Cowboy daemon.
The Zed crates and wire schema used by remote development are
GPL-3.0-or-later. They therefore live in a separately built and distributed
`cowboy-zed-adapter` process. Its source is the `zed` plugin under
`plugins/zed`; `plugin.json` pins the shared code-intelligence contract and the
adapter remains its isolated entry point.

## Version boundary

The adapter pins one Zed source revision and one matching remote-server build.
The initial compatibility target is:

- remote server version: `1.13.0`;
- Zed revision: `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`.

An upgrade changes both values together, bumps the Zed plugin version, and must
pass the adapter protocol fixtures plus `just plugin-check` before deployment.
Cowboy never discovers or executes the mutable
server under `~/.zed_server`. New Plugin releases own the exact adapter/server
pair and its typed launch/readiness graph. The package's Nix recipes under
`plugins/zed/runtime` build a static distributable Linux adapter; a regular
Nix bootstrap binary is not a portable Plugin artifact.

The browser's read-only Code syntax fallback carries a snapshot of the pinned
Zed `first_line_pattern` matchers, identified by the same revision. It applies
those matchers only after path-based detection, so a shebang can identify an
extensionless file without overriding a known filename. Updating the Zed pin
must update this snapshot and its routing fixtures together. Project-specific
`file_types` settings remain a Zed-side capability and are not guessed by the
browser.

## Process and state isolation

There is one adapter instance per active Plugin generation on a Machine,
shared by that generation's Code worktrees. A worktree with live buffer or
worktree leases retains its generation across upgrade and uninstall. Other
worktrees can start against a newer installed generation; no shared socket or
global server path is replaced underneath a leased worktree. Runtime death
reports lost protocol state rather than replaying against an ambient binary.

Legacy opens reserve their original route before dispatch, so a lost reply or
cancelled caller cannot make a possibly held buffer look idle during a Plugin
upgrade. A buffer reservation ends only after that owner's close is confirmed.
An unconfirmed non-idempotent worktree open also pins its runtime and refuses
another open; a later ensure/close observation does not prove that the original
handler finished. This conservative pin ends with runtime loss; it is not
independent worktree-open recovery.

Each generation has a private home below
`plugins/<id>/runtime/<digest>/home`. Each live process group has a fresh 0700
Unix-socket/state directory with a short path, separate from that home. The
Machine removes only that owned temporary directory on teardown. No ordinary
Zed settings, caches or credentials are inherited.

Pre-migration bootstrap services retain their older separate directories:

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
or formatter commands without the owner's grant. The grant is the
"Project language servers" directory default (`language_servers`), inherited
live through the session's directory like the Review directory; unset means on,
an explicit `off` restricts. The Controller sends that value as `trusted` and,
when it is off, reports `restricted` and refuses language reads and new
buffers even if the adapter still holds the worktree trusted from before.
Since Zed Plugin `1.21.0` turning it off also revokes native trust: the adapter
restricts the worktree at once and refuses every language query on it
(including owned-buffer reads), and once no lease or buffer remains it removes
the native worktree, which drops the language servers rooted only there, and
adds it back restricted. `RestrictWorktrees` alone would not stop them at the
pinned revision. Older adapters keep trust until their last lease ends. Restricted worktrees still provide files, Git state, and
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

The first production slices implement idempotent worktree ensure plus leased
worktree and read-only buffer open/close operations against the pinned Zed
server. Cowboy's manifest reports `provider: "zed"` and `state: "ready"` only
after the ensure response succeeds; individual LSP capabilities remain false
until their stable adapter messages are implemented.
