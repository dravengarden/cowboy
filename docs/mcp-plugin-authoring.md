# Authoring a third-party MCP Plugin

Cowboy runs MCP servers that it did not write. A publisher outside this
repository builds, signs and publishes an `mcp_server` Plugin; an operator
installs it on a Machine; a session owner grants it to one session. Each step
re-verifies the signed bytes. [MCP bindings](mcp-bindings.md) describes the
runtime contract; this page is the publisher's procedure.

## What the publisher ships

A Plugin directory with:

- `plugin.json`: `kind: "mcp_server"`, `entrypoint: "contract.json"`, an exact
  `component_release` and component pins including `cowboy.plugin-sdk` 1.13.0
  or later. `publisher` is a lowercase kebab-case name that is not
  `cowboy-first-party`.
- `contract.json` (`McpServerContract`): the `stdio_session_v1` protocol, the
  `command` and fixed `arguments`, the tool names, optional signed `discovery`
  (tool schemas served before the server starts) and optional `sources`.
- `runtime-artifacts.json`: one self-contained artifact per platform. Its URL is
  `https://<controller>/plugin-artifacts/<sha256>/<file>`; the digest in the
  path must equal the artifact digest.

The contract holds no endpoint, environment or credential. The server starts
with an empty environment and one variable, `COWBOY_MCP_PROFILE`, a private
`/proc/<pid>/fd/<n>` path to the session's immutable process profile. A server
must refuse to run outside that profile rather than fall back to local mode.

## Sources

`sources` names external systems the server can read (for example `github`).
A session grant selects a subset; the profile carries it as
`binding.sources`. The server must refuse an ungranted source before reaching
it. A granted source uses credentials already provisioned for the execution
Machine's user. Cowboy never delivers credentials to the server.

## Build, sign and publish

With the packaged `cowboy-plugin-pack` and the publisher's Ed25519 key (keep
the private key outside every repository):

```sh
cowboy-plugin-pack build SRC OUT/ID.cowboy-plugin
digest=$(jq -r .package_digest OUT/ID.release.json | cut -d: -f2)
cowboy-plugin-pack set-artifact-url OUT/ID.cowboy-plugin OUT/ID.release.json \
  "https://<controller>/plugin-artifacts/$digest/ID.cowboy-plugin"
cowboy-plugin-pack bind-runtime OUT/ID.cowboy-plugin OUT/ID.release.json SRC/runtime-artifacts.json
cowboy-plugin-pack sign OUT/ID.cowboy-plugin OUT/ID.release.json PRIVATE_KEY
cowboy-plugin-pack verify OUT/ID.cowboy-plugin OUT/ID.release.json PUBLIC_KEY
```

Place each runtime artifact at `OUT/runtime/<os>-<arch>/<command>[.tar.gz]`.
On the Controller host, as its Service account:

```sh
bun tools/publish-plugin-release.ts ID /var/lib/cowboy/plugins/catalog PUBLIC_KEY OUT
```

The first publication records `trusted-publishers/<publisher>.pub`; a later
release with a different key for the same publisher is refused. Publication
is immutable: a version names one release forever.

## Install and grant

```sh
cowboy operator refresh-catalog
cowboy operator install --machine hawk --plugin ID --version V \
  --digest sha256:<artifact digest> --operation-id <stable id>
```

Installation only makes the exact generation available. A session receives it
through `PUT /api/sessions/{id}/mcp-bindings` with the plugin ID, generation
digest, a tool subset and a source subset, and only on a Provider that declares
`provider.mcp-bindings.v1`.
