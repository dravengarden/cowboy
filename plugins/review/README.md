# Review call profile

Review 1.0.6 is a Cowboy `call_profile` Plugin. It declares the purposes of
Cowboy managed agent calls (code review, design review, analysis) and the
agent-facing guidance that tells a session when and how to request a
read-only review from the other agent family. `{{peer}}` and
`{{peer_command}}` are filled by the calling Provider.

Grants, workspace snapshots, the read-only sandbox and placement remain
Cowboy core; this package is data only. Agents prefer Cowboy MCP, discover
operation help on demand and retain CLI fallback. Review remains optional;
this package adds no per-turn review or polling: agents start a review with
`notify:true` and end the turn until Cowboy posts its completion notice. See
[session extension Plugins](../../docs/session-extension-plugins.md) and
[managed agent calls](../../docs/managed-agent-calls.md).

Build from the repository's pinned shell:

```sh
nix develop -c just plugin-build review
```
