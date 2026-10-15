# Matrix memory backend

Matrix 1.0.0 declares the Matrix shared-memory service as a Cowboy
`memory_backend` Plugin: the `matrix_http_v1` protocol driven by
`@cowboy/memory-client`, the seven exact MCP tools a Provider may expose, and
that the service answers recall and accepts observations.

No endpoint, token, project mapping or executable is packaged. The binding is
private to the Machine user that runs the Provider, at
`$XDG_CONFIG_HOME/cowboy/memory/matrix/<provider>.json` (mode 0600); see
[session extension Plugins](../../docs/session-extension-plugins.md).

Build from the repository's pinned shell:

```sh
nix develop -c just plugin-build matrix
```
