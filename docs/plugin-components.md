# Plugins and shared components

Cowboy organizes independently versioned integrations as plugins and reusable
implementation as components. The machine-readable sources of truth are
`plugins/*/plugin.json`, `components/plugin-contract/schema.json`, and
`components/registry.json`.

## Plugin boundary

A plugin manifest has an exact ID, SemVer version, kind, entry point, and exact
component dependency list. The initial kinds are:

- `agent_provider`: an installable agent integration whose entry point is the
  existing signed, data-only Provider package contract;
- `code_intelligence`: an isolated code-intelligence integration. The first is
  the separately built GPL Zed adapter.

The generic Plugin identity is the repository, publication, discovery,
installation, rollback, and uninstall boundary. A Provider package is only the
typed payload of an `agent_provider` Plugin: it has no independent signature,
Catalog, installation slot, or Machine lifecycle. Zed uses the same signed
Plugin generation lifecycle while remaining process-isolated rather than an
in-process Rust dependency.

## Component boundary

Components contain reusable implementation or contracts, never an independently
installed product. Every active component is also a distributable package:
Cargo crates for Rust SDKs and npm source packages with explicit exports for
TypeScript, schemas, and runtime tooling. Cowboy consumes TypeScript components
by package name rather than reaching into their source directories. The
registry includes the plugin contract, Web app shell, reactive store,
optimistic sync and IndexedDB adapter, Provider SDK/UI/runtime tooling, and the
Zed code-intelligence contract. Cowboy no longer stages or imports
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
without creating another component release or changing any sibling Plugin.
When a component release changes, every Plugin must still increase its version,
preserving the coordinated compatibility rule.

`cowboy-plugin-pack build` reads the Plugin manifest's `component_release`; it
does not read Cowboy's component registry or assume a Cowboy checkout as its
working directory. `just plugin-isolation-check <id>` executes that build from
an unrelated temporary directory. This is the repository acceptance proof for
moving a Plugin to its own repository and release pipeline.

## Coordinated release rule

`components/registry.json` is an append-only release history. If any registered
component source changes, its digest and version must change in a new component
release. Every plugin listed by the preceding release must then receive a
strictly higher SemVer version, including plugins that do not directly consume
the changed component. This intentionally makes the tested Cowboy plugin set a
coherent release train and prevents a shared-component upgrade from silently
leaving one plugin on an unverified combination.

Run:

```sh
just plugin-check
just plugin-build <plugin-id>
```

`plugin-check` validates publishable package manifests and exports, source
digests, exact dependency pins, plugin and entry-point identity, Provider
payload versions, the Zed adapter version, and the all-plugins-bump rule between
adjacent component releases. `provider-check` additionally proves a Plugin can
build from an unrelated working directory. The repository-wide `just check`
includes both gates.

## Layout

```text
components/
  registry.json
  plugin-contract/
  plugin-sdk/
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
```

Machine Plugin state lives under `plugins/`. Startup atomically adopts the old
`providers/` root only when the new root does not yet exist. Provider-named auth
replica paths remain capability-specific state, not an extension lifecycle.
