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
installed product. The initial registry includes the plugin contract, Web app
shell, reactive store, optimistic sync, Provider SDK/UI/runtime pins, and the
Zed code-intelligence contract. Web components now live under
`web/src/components`; Cowboy no longer stages or imports `shared-utils`.

Each component release records:

- an exact component version;
- every source path owned by that component;
- a deterministic SHA-256 digest of those sources.

Plugins pin exact component versions. Ranges, implicit workspace versions, and
moving references are invalid.

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

`plugin-check` validates source digests, exact dependency pins, plugin and entry
point identity, Provider payload versions, the Zed adapter version, and the
all-plugins-bump rule between adjacent component releases. `provider-check` and
the repository-wide `just check` include this gate.

## Layout

```text
components/
  registry.json
  plugin-contract/
  provider-sdk/
  provider-ui/
  provider-runtime-*/
  code-intelligence/
plugins/
  <agent-provider>/plugin.json + provider.json
  zed/plugin.json + adapter/
web/src/components/
  app-shell/
  state/store/
  state/sync/
  state/sync-idb/
```

Machine Plugin state lives under `plugins/`. Startup atomically adopts the old
`providers/` root only when the new root does not yet exist. Provider-named auth
replica paths remain capability-specific state, not an extension lifecycle.
