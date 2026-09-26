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
`OwnedResourceScope` and state-sync owner barriers. [Owned IDB connections](owned-idb-connections.md)
add database owners and transaction leases. [Atomic outbox deltas](atomic-idb-outboxes.md)
now preserve updated peers' pending mutations; old/new-client lifetime fencing,
general host migration and durable execution/recovery remain pending. This gate grants no
runtime authority. The later [core Web host slice](core-web-plugin-host.md)
removes the application's dependency on the mixed Plugin runtime and adds typed
slots with owned inventory observations. Published SDK/native ABI retirement and
production security-policy acceptance remain pending.

[Provider UI authoring and ownership](provider-ui-ownership.md) adds the pure
`@cowboy/provider-authoring` contract and a separate core interactive-view owner.
Field/message links are checked at compile time; admission, permissions and
asynchronous lifetime remain core responsibilities, never an installable UI host.

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

After that baseline, adding component identities requires an explicit nonempty
`component_additions` list on the new schema-3 release. It must equal exactly the
new IDs: undeclared additions, removals, duplicate/reused identities, missing
nodes and cycles fail. New dependency edges still version every affected
existing component and Plugin. Release 3.4.0 uses this additive migration for
`cowboy.provider-authoring` only; historical records and all seven Plugin pins
remain unchanged. This build-metadata migration is not runtime authority.

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

Scoped release 3.29.0 records provider-runtime 1.1.7. Both DeepSeek gateways
advance to Columbus `27352e34`: `codex-deepseek` 0.3.0 serves `deepseek-flash`
(DeepSeek-V4.1-Flash, with image input), its retained `deepseek-v4-flash` alias
and `deepseek-v4-pro` through DeepSeek's native Responses API and deletes the
temporary Pro Chat adapter; `claude-deepseek` 0.1.1 only changes content-free
telemetry. Both DeepSeek Plugins move their presets, defaults and runtime model
names to DeepSeek's recommended `deepseek-flash`. Every other pin is unchanged,
but the shared lock is part of every Agent Plugin's closure, so all six take the
new closure and independent patch versions; Zed again retains its unchanged
source, version, and historical component-release pin.

Scoped release 3.28.0 records provider-runtime 1.1.6. Codex advances from 0.154.0
to 0.156.1 and Codex ACP from 1.11.0 to 1.13.1, including the adapter's exact ACP
SDK 1.5.0 lock; Grok advances from 1.0.30 to 1.0.41. The Codex CLI release adds
GPT-6 Sol and Luna to its model catalog, and every pinned preset model
(`gpt-5.6-sol`, `gpt-5.6-luna`, `gpt-6-astra`) still resolves, so the presets
stay as published. The one Codex runtime lock entry per component id is shared, so
`codex-deepseek` takes the same pins as `codex`; all six Agent Plugins take the
new closure and independent patch versions, and Zed again retains its unchanged
source, version, and historical component-release pin.

Scoped release 3.27.0 records provider-runtime 1.1.5. Claude Code advances from
2.1.278 to 2.1.280 and Claude Agent ACP from 0.79.0 to 0.81.0, including the
adapter's exact Agent SDK 0.3.280 lock. The CLI release adds Claude Opus 5.5
(`claude-opus-5-5`), so the Agent model list gains it without a Cowboy contract
change. Both Anthropic Providers share the one runtime lock entry per component
id, so `claude-deepseek` moves with `claude-code`; all six Agent Plugins take the
new closure and independent patch versions, and Zed again retains its unchanged
source, version, and historical component-release pin.

Scoped release 3.26.0 records provider-runtime 1.1.4. Claude Code advances from
2.1.272 to 2.1.278 and Claude Agent ACP from 0.77.0 to 0.79.0, including the
adapter's exact Agent SDK 0.3.274 lock. All six Agent Plugins move to the new
runtime component closure and receive independent patch versions; Zed retains
its unchanged source, version, and historical component-release pin.

The first scoped release changes only `state-store` (2.0.0), `state-sync`
(1.3.0) and `state-sync-idb` (1.3.0). All seven Plugin sources and their exact
2.9.0 pins are unchanged. Their signed packages, private runtimes, Catalog
formats and Machine installations are untouched. The Controller understands both
registry schemas and projects each Plugin's own release label; the registry is
embedded build metadata, so an old Controller/rollback binary still reads its
own matrix and the same signed Catalog. No new Catalog envelope or reader bridge
is needed.

Scoped release 3.3.0 adds state-sync-idb 1.5.0's explicit database owner, bounded
open results, transaction-terminal leases and exact state-store 2.1.0 peer.
The seven Plugin releases and their 2.9.0 pins remain unchanged.

Scoped release 3.5.0 adds state-sync 1.5.0's exact load-result handoff and
state-sync-idb 1.6.0's atomic outbox deltas. Database schema/record bytes and
all seven Plugin sources/pins remain unchanged; old blind writers still require
an upgrade. This core-only slice needs a Web release, not a Catalog transaction.

Scoped release 3.6.0 adds state-sync-idb 1.7.0's exact schema floor, optional
transaction-lifetime handles and strict bounded inspection. The core
[product dataset integration](product-sync-datasets.md) binds immutable
Service/user identity to Web persistence and authenticated socket negotiation;
its Controller/Web migration and recovery acceptance are independent of the
unchanged seven Plugin sources, pins and signed Catalog.

Scoped release 3.2.0 adds state-store 2.1.0's owned resource scope, state-sync
1.4.0's lifetime/barrier fixes, and the state-sync-idb 1.4.0 transitive peer
update. The same seven Plugin releases and exact 2.9.0 pins remain unchanged;
the [local lifecycle slice](owned-component-scopes.md) requires only a Web
activation, not a Catalog write or Machine installation.

Scoped release 3.10.0 records app-shell 1.1.2's Android system-haptic bridge
after integrating the native-shell change. No component or Plugin depends on
app-shell, so the release changes only that component; existing Plugin source,
versions and component-release pins remain unchanged. The matrix snapshots the
current independently versioned Plugins without rewriting historical entries.
This metadata repair does not publish a Catalog or activate a native shell.

Scoped release 3.12.0 records app-shell 1.1.3's already-integrated connection
banner change. App-shell has no component or Plugin consumers in the closure,
so only its package version/digest changes. The current Plugin sources are
snapshotted, including the independently versioned Zed 1.18.0 candidate; their
historical component pins and existing signed releases remain unchanged.

Scoped release 3.25.0 records app-shell 1.1.16: the update bar shows its press
as a control. A full-width tinted bar at the top of a screen already means
"notice", so the imperative sentence inside it did not read as a button; the
verb now leaves the sentence and is drawn as an outlined `UpdateActionPill`
naming what the next press does. The pill is decoration — `pointer-events:
none` and `aria-hidden` — so the target stays screen-wide and a screen reader
still hears one control. Only app-shell's version/digest changes; it still has
no component or Plugin consumers, and no Plugin manifest, signed bytes or
component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.24.0 records app-shell 1.1.15: a download nobody asked for is
a translucent hairline at the top edge rather than a bar of text the user can
only watch, and the bar arrives with the thing it announces
(`updateShowsHairline`, `updateHairlineSx`). `UpdatePhase` gains `rejected`
and `abandoned` for a build this device watched fail to start: the worker
serves the previous generation again, stops promoting that deploy in a
version-scoped state cache, and the bar carries a warning-toned notice whose
press lifts the rejection. `useAutoUpdate` gains `beforeReload` so the app can
record the swap its next boot has to sign for. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.23.0 records app-shell 1.1.14: the shell-refresh progress
added in 3.22.0 is opt-in. That reply port's other caller is the previous
build's client, which resolves on the first message it receives and reads one
without `ok` as a failed download, so an unasked progress message stranded
every open older page on "could not be downloaded yet · retrying" against a
download that had in fact succeeded. The worker now streams batches only to a
client that sends `progress: true`, and an unflagged request keeps its exact
single-reply shape. Only app-shell's version/digest changes; it still has no
component or Plugin consumers, and no Plugin manifest, signed bytes or
component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.22.0 records app-shell 1.1.13: the update banner is now the
update control. The deployed build is downloaded as soon as a deploy is
detected and the service worker streams its boot-asset count back, so the bar
fills with a real download and a press swaps builds from cache alone
(`downloadUpdate`/`reloadIntoUpdate` replace `applyUpdate`; `useAutoUpdate`
gains `phase`, `progress` and `requestUpdate`, and `update-presentation.ts`
owns the fill). The automatic countdown is unchanged and still arrives for a
user who never presses. Only app-shell's version/digest changes; it still has no
component or Plugin consumers, and no Plugin manifest, signed bytes or
component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.21.0 records app-shell 1.1.12: a zoomed preview figure moves
like a scroll view instead of a dragged box. A released pan now coasts its
smoothed speed out on one compositor transition whose curve leaves at exactly
the speed the finger did, an axis the figure already fits is rigid rather than
elastic — a wide Mermaid diagram no longer drifts vertically while it is panned
sideways — a settle keeps its pan layer promoted instead of demoting it on the
frame the animation starts, and a finger landing mid-settle freezes the figure
where it is painted rather than jumping to the target. The same freeze fixes a
second zoom step taken mid-animation, which used to measure a stale rect and
bake a size the figure never had. Only app-shell's version/digest changes; it
still has no component or Plugin consumers, and no Plugin manifest, signed bytes
or component pin changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.20.0 records app-shell 1.1.11: a deployed build is applied by
the client itself on every surface. `useAutoUpdate` and the pure
`update-policy` rules move the countdown out of the desktop banner so the phone
runs the same policy, add the foreground dwell a resumed PWA needs, and re-arm
the one-second check that a held countdown previously dropped — a busy page
could stay on the old build until it was reloaded by hand. Only app-shell's
version/digest changes; it still has no component or Plugin consumers, and no
Plugin manifest, signed bytes or component pin changes. No Catalog publication
or Machine upgrade is needed.

Scoped release 3.19.0 records app-shell 1.1.10: a fullscreen preview also takes
the document background and colour scheme to its backdrop while it is open. The
iOS standalone status bar sits above the web view and a light iPhone PWA fills
that strip from the document, not from `theme-color`, so the bar stayed white
over a near-black preview; all three writes are snapshotted and restored on
close. Only app-shell's version/digest changes; it still has no component or
Plugin consumers, and no Plugin manifest, signed bytes or component pin
changes. No Catalog publication or Machine upgrade is needed.

Scoped release 3.18.0 records app-shell 1.1.9: the lightbox commits the neutral
paint when it swaps the pan layer's layout size. A CSS transition starts from
the previous style recalculation, so baking (or unbaking) the layout and
starting an animated transform in the same task interpolated the old transform
against the new layout — a 3x figure flashed to 9x and eased back over the
settle, the twitch at the end of a pinch. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.17.0 records app-shell 1.1.8: the lightbox's baked pan layer
scales the plate's padding with it, so settling a pinch no longer widens the
content box and twitches the artwork, and a fullscreen preview takes the iOS
standalone status bar to its backdrop colour (through the sheets' own
`setStatusBarColor`, now shared) and restores the app's colour on close. Only
app-shell's version/digest changes; it still has no component or Plugin
consumers, and no Plugin manifest, signed bytes or component pin changes. No
Catalog publication or Machine upgrade is needed.

Scoped release 3.16.0 records app-shell 1.1.7: the lightbox writes its plate
onto an inline-SVG figure's own element. A host renderer paints a background on
the SVG root, and that inline style beat the component's class rule, so a light
figure lost its white plate and its dark strokes disappeared into the near-black
backdrop. Only app-shell's version/digest changes; it still has no component or
Plugin consumers, and no Plugin manifest, signed bytes or component pin changes.
No Catalog publication or Machine upgrade is needed.

Scoped release 3.15.0 records app-shell 1.1.6: a zoomed lightbox figure keeps
`flexShrink: 0`, so the baked pan layer is no longer shrunk back to the viewport
by the centring flex row — an inline SVG diagram stayed pinned at fit size
because its automatic flex minimum is zero. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.14.0 records app-shell 1.1.5: the connection banner's update
action now downloads the deployed shell through the service worker before it
reloads and reports whether that finished (`applyUpdate(): Promise<boolean>`),
so a weak connection keeps the running build. Only app-shell's version/digest
changes; it still has no component or Plugin consumers, and no Plugin manifest,
signed bytes or component pin changes. No Catalog publication or Machine
upgrade is needed.

Scoped release 3.13.0 records app-shell 1.1.4's already-integrated gallery image
contract. Only app-shell's version/digest changes; it still has no component or
Plugin consumers. Current independently released Claude Code 3.1.28 and Zed
1.20.0 sources are snapshotted without changing their manifests, signed bytes
or 3.11.0 component pins. No Catalog publication or Machine upgrade is needed.

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
