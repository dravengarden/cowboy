# Plugins and shared components

Cowboy organizes independently versioned integrations as plugins and reusable
implementation as components. The machine-readable sources of truth are
`plugins/*/plugin.json`, `components/plugin-contract/schema.json`,
`components/plugin-contract/authentication-provider.schema.json`,
`components/code-intelligence/schema-v2.json`, and `components/registry.json`.

The target [spatiotemporal design, section 2.3](plugin-spatiotemporal-design.md)
places component libraries within one cross-Service/Machine composition model:
pure utilities, core implementations, authoring contracts, private Plugin
implementations and templates. Owned units do not acquire an installation
identity. The schema-3 dependency-closure release gate and state-store's typed,
owned persistence implement the first component slice of sections 9–10.
[Owned component scopes](owned-component-scopes.md) now add process-local
`OwnedResourceScope` and state-sync owner barriers; general host/IDB connection
migration and durable execution/recovery remain pending. This gate grants no
runtime authority. The later [core Web host slice](core-web-plugin-host.md)
removes the application's dependency on the mixed Plugin runtime and adds typed
slots with owned inventory observations. Published SDK/native ABI retirement and
production security-policy acceptance remain pending.

## Plugin boundary

A plugin manifest has an exact ID, SemVer version, kind, entry point, and exact
component dependency list. The initial kinds are:

- `agent_provider`: an installable agent integration whose entry point is the
  existing signed, data-only Provider package contract;
- `authentication_provider`: a signed, data-only product-login integration.
  Payload schema 1 selects OIDC; schema 2 also selects local password and
  WebAuthn. The Controller owns credentials, account mappings, protocol
  execution, and Cowboy sessions; the package cannot execute provider code;
- `code_intelligence`: an isolated code-intelligence integration. The first is
  the separately built GPL Zed adapter and its exact private server. Schema 2
  owns both the executable graph and launch/readiness bindings; schema 1 is
  retained for previously published legacy adapters.

The generic Plugin identity is the repository, publication, discovery,
installation, rollback, and uninstall boundary. A Provider package is only the
typed payload of an `agent_provider` Plugin: it has no independent signature,
Catalog, installation slot, or Machine lifecycle. Zed uses the same signed
Plugin generation lifecycle while remaining process-isolated rather than an
in-process Rust dependency.

## Component boundary

Components contain reusable implementation or contracts, never an independently
installed product. Every active component is also a distributable package: Cargo
crates for Rust SDKs and npm source packages with explicit exports for
TypeScript, schemas, and runtime tooling. Cowboy consumes TypeScript components
by package name rather than reaching into their source directories. The registry
includes the plugin contract, versioned plugin host API, Web app shell, reactive
store, optimistic sync and IndexedDB adapter, Provider SDK/UI/runtime tooling,
and the Zed code-intelligence contract. Cowboy no longer stages or imports
`shared-utils`.

Each component release records:

- an exact component version;
- every source path owned by that component;
- a deterministic SHA-256 digest of those sources.
- its Cargo/npm package name and package manifest.

Plugins pin exact component versions and the exact component release. Ranges,
implicit workspace versions, private packages, missing public exports, and
moving references are invalid.

## Independent Plugin releases

The component registry records the minimum Plugin version tested when a shared
component release is cut. A Plugin may subsequently increase its own version
without creating another component release or changing any sibling Plugin. When
a component release changes, schema 3 versions the affected dependency closure.
An unaffected Plugin keeps its exact source, version and historical
component-release pin; the active matrix is not a new identity for that Plugin.

`cowboy-plugin-pack build` reads the Plugin manifest's `component_release`; it
does not read Cowboy's component registry or assume a Cowboy checkout as its
working directory. `just plugin-isolation-check <id>` copies the Plugin source
into an unrelated temporary directory and builds there. `provider-check`
exercises all seven first-party Plugins. This is the acceptance proof for moving
a Plugin to its own repository and release pipeline.

External publishers can use `nix build .#cowboy-plugin-pack` to obtain a
closure-pinned SDK CLI, including its signature tool. The `version` command
reports the exact SDK version; no Cowboy working directory is needed to build an
external package. Cardea's repository owns its own release recipes using this
tool, rather than copying the SDK or relying on an ambient compiler.

## Dependency-closure release rule

`components/registry.json` remains append-only. Schema-2 entries (through 2.9.0)
still enforce strictly increasing versions for every Plugin between adjacent
releases. Schema 3 appends a separate, unchanged 3.0.0 migration baseline before
the first dependency-scoped change in 3.1.0. It never reinterprets old entries.

Each new release records all internal component-package edges and each Plugin's
exact source digest, component pins and component-release label. Plugin source
digests use the Git source list (including new non-ignored source before
commit), excluding local Cargo/Node build caches. Isolated builds use that same
list and compare package/host bytes with the in-tree SDK build, so an ignored
input cannot silently affect a published package. npm dependency, peer, optional
and development pins are checked separately; Cargo edges come from offline,
locked `cargo metadata`. Every declared internal edge is currently
conservatively release-causing, including contract/build inputs. Fine-grained
contract compatibility exemptions are not implemented. The matrix itself is
tested compatibility metadata, not an implicit dependency on every component.

A changed component requires a higher version, as do its transitive package
consumers. Missing nodes, cycles, ranges, stale pins, incomplete package source
coverage and symlinked release sources fail the gate. A Plugin may retain an old
component-release label only if its whole declared transitive closure has
identical component versions, digests, source ownership and package identities
in that release and the active matrix. A changed Plugin source snapshot or
binding requires a higher Plugin version. Adding/removing component or Plugin
identities still requires an explicit migration, not an inferred exemption.

The first scoped release changes only `state-store` (2.0.0), `state-sync`
(1.3.0) and `state-sync-idb` (1.3.0). All seven Plugin sources and their exact
2.9.0 pins are unchanged. Their signed packages, private runtimes, Catalog
formats and Machine installations are untouched. The Controller understands both
registry schemas and projects each Plugin's own release label; the registry is
embedded build metadata, so an old Controller/rollback binary still reads its
own matrix and the same signed Catalog. No new Catalog envelope or reader bridge
is needed.

Scoped release 3.2.0 adds state-store 2.1.0's owned resource scope, state-sync
1.4.0's lifetime/barrier fixes, and the state-sync-idb 1.4.0 transitive peer
update. The same seven Plugin releases and exact 2.9.0 pins remain unchanged;
the [local lifecycle slice](owned-component-scopes.md) requires only a Web
activation, not a Catalog write or Machine installation.

Run:

```sh
just plugin-check
just plugin-build <plugin-id>
```

`plugin-check` validates publishable package manifests and exports, source
digests, exact dependency pins, plugin and entry-point identity, Provider
payload versions, the Zed adapter version, legacy coordinated history, and new
dependency-closure transitions. `provider-check` additionally proves the Plugins
can build from isolated source copies. The repository-wide `just check` includes
both gates.

For release preparation, `--print-digests` and `--print-closure` print candidate
metadata only; they are not validation or publication commands. Normal
`deno run --allow-read --allow-run tools/check-plugin-components.ts` validates
the complete tree in the pinned shell (Cargo metadata inherits that shell).

## Owned preference state

`@cowboy/state-store/core` has no React dependency. v2 requires explicit typed
serialize/deserialize functions and prevents a decoder from widening the store's
inferred type. Existing Web codecs preserve preference keys and encodings.
Listeners belong to subscriptions, with release on last unsubscribe; owned
instances additionally expose idempotent `dispose()`. Delayed callbacks are
fenced across listener/instance lifetimes. Failed storage access cannot leave a
committed value invisible to its subscribers. See the
[component contract](../components/state-store/README.md) for fallback
semantics. View cleanup neither deletes preferences nor cancels Machine work.

`just example-auth-build-all` discovers every login host under
`examples/authentication/` and requires its own manifest and SDK-only package
build. The examples include release-ready Password and Passkey source packages;
they are not additional Machine installations or an alternative release format.
They join `provider-check` so a shared SDK change exercises the local-login
migration prerequisites as well as the seven Machine Plugins.

## Layout

```text
components/
  registry.json
  plugin-contract/
  plugin-sdk/
  plugin-api/
  app-shell/
  state-store/
  state-sync/
  state-sync-idb/
  provider-sdk/
  provider-ui/
  provider-runtime/
  code-intelligence/
plugins/
  <agent-provider>/plugin.json + provider.json
  zed/plugin.json + adapter/
examples/authentication/
  <authentication-provider>/plugin.json + authentication.json
```

Machine Plugin state lives under `plugins/`. Startup atomically adopts the old
`providers/` root only when the new root does not yet exist. Provider-named auth
replica paths remain capability-specific state, not an extension lifecycle.
