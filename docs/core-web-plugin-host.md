# Core-owned Web Plugin presentation

The fourteenth spatiotemporal slice removes the production Web application's
dependency on the mixed `@cowboy/plugin-api` runtime. It changes no signed
Plugin package, host wire schema, native ABI, Controller policy or Machine.
The old SDK remains an immutable compatibility input for existing releases,
not the Web's registry or native authority. This is a P1 implementation slice,
not completion of P1 or the cross-site executor.

## Boundary and types

`web/src/pluginHost/` owns the closed projection reader, per-instance observation
state, view contracts and React slot. `web/src/pluginHost.ts` selects compiled
Cowboy renderers directly. There is no exported registration API, Plugin JS
loader, generic native dispatch or `context?: unknown`. Authoring and validation
of signed Provider UI data still use the existing Provider SDK.

The observation component does not import domain renderer implementations.
The login page supplies its core-local OIDC presentation; Provider views use a
separate compiled wrapper. This avoids pulling Provider panels, stores and
sheets into the unauthenticated login tree. A core render callback is not a
Plugin registration or a wire capability. Static-render fixtures use an explicit
snapshot and acquire no observation subscription.

The slot tag determines its entire context: OIDC login, Provider card, usage, or
the matching setup/empty/settings lifecycle. Lifecycle slot, context kind and
surface must agree. Effects retain the closed `EffectSchema` callback type;
callbacks are core-local references, never serialized Plugin data. Exact host
selection pairs version and digest; an incomplete/invalid pair cannot silently
select a default. `contracts.typecheck.ts` contains compile-negative assertions
and is included in the production strict TypeScript gate.

The reader accepts the current legacy slot/renderer vocabulary without mounting
Password/Passkey ceremonies from Plugin declarations. Native claims are checked
for compatibility and stripped from the downstream projection. The native port
remains the separate `coreNativeBridge.ts`; no native ABI is retired here.

## Space and observation lifetime

Each `createPluginHostInventory()` owns its maps, read tokens and subscriptions.
`webPluginHosts` is the explicit Web-root instance. Construction installs no
global listener, timer, fetch or native operation. Root startup owns one
session-end listener with an idempotent disposer; hot disposal releases it.
Each view owns only its subscription, including separate registrations using
the same callback. A failed observer cannot block the others.

The two complete observations are independent:

- `/api/auth/status` owns external login presentation. An empty response removes
  old auth rows rather than merging them forever.
- `/api/plugins` owns Provider presentation, usage, colors and occupancy. Auth
  refresh cannot replace a Provider generation or select its renderer.

The enclosing Catalog/auth response validates before its host observation
changes. Host parsing has closed public fields, version/generation checks,
unique identities/slots, slot-to-renderer checks and size/depth bounds. Exact
release digests must match the projected generation. Snapshots are detached and
frozen. Duplicate exact identities disappear; ambiguous defaults are disabled
in resolution and downstream metadata alike. This is not client-side signature
verification: the authenticated Controller still verifies packages and owns
the execution authorization decision.

## Time and race handling

Each source has at most one current read token and AbortController. Replacing a
read, ending the product session or disposing the owner aborts observations and
invalidates their identity. A transport ignoring abort still cannot commit a
stale response. Tokens belong to one inventory, are consumed once and are not
deserializable grants. Failed reads release their registration without erasing
the last accepted observation. A stale Catalog request's `finally` cannot clear
a new session's pending request.

`PluginSlot` synchronously resolves a snapshot through `useSyncExternalStore`;
it makes no network request. Replacements update already mounted views rather
than leaving a previously loaded renderer attached indefinitely. The error
boundary/child incarnation is keyed by exact host identity, generation, slot
and renderer; an unrelated refresh does not remount the same binding. The core
card shell now uses its fallback directly instead of invoking a lifecycle
renderer with an incompatible context and catching an exception on every card.

Session end also clears Catalog and usage/color/occupancy projections. None of
this cleanup uninstalls a Plugin, cancels a submitted operation, logs out a
Provider account, deletes durable state or stops a Machine worker. It removes
Web observations, not Service authority. Existing trusted core fallbacks remain
subject to normal server-side authorization.

## Verification and remaining work

Tests cover exact coexistence, mounted-observer replacement, duplicate and
ambiguous identities, invalid projections/envelopes, detached and bounded data,
separate auth/Catalog ownership, old/new response ordering, session cleanup,
pending-request identity, cross-owner/single-use tokens, reentrant reset, failed
observers and idempotent disposal. A whole-production-Web source check rejects
legacy SDK/native-global dependencies; a behavioral test proves changing the
old SDK registry cannot change the core inventory. Existing auth, native-port,
Provider and package gates still apply. These tests do not constitute physical
device or actual React DOM/native acceptance.

Release only the Web lane with a new PWA cache version. Existing Plugin versions,
component history, signed bytes and Catalog reader floors remain unchanged.

Remaining P1 work includes public authoring SDK/Provider IR field-message
correlation, broader component resource scopes, production CoreSecurity policy
handoff and true native/client acceptance before old SDK/native ABI retirement.
P2–P4 execution and durable compensation remain separate substantive work.
