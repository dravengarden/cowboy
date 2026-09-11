# Provider UI authoring and core-owned execution

This is the seventeenth verified slice of the
[spatiotemporal Plugin design](plugin-spatiotemporal-design.md), not completion
of P1 or a distributed DAG executor.

## Separate authoring from execution

`@cowboy/provider-authoring` 1.0.0 is a pure data authoring component with an
exact peer on `@cowboy/provider-ui` 3.1.12. `defineProviderUiContract` infers a
literal `{ logic, ui }` and links state initial values, message payloads,
reducer targets/sources, effect/completion references, state/asset references,
conditions and surface effect ownership. `ProviderStateOf` and
`ProviderMessageOf` expose the inferred state and discriminated messages. Actual
button payloads reject missing/extra keys, even for empty schemas and non-fresh
objects. References cannot widen declarations through generic inference; a
widened parsed IR is not an authoring proof.

The helper returns the same wire object. It does not validate downloaded JSON,
grant capabilities, register renderers or execute a Plugin. Safe integers,
duplicate declarations, schema/resource bounds and fingerprints still require
the independent SDK validators. Type casts and structural typing are not
security. The strict Web typecheck compiles positive and negative fixtures;
runtime tests also pass the fixture through the existing UI validator.

Registry 3.4.0 explicitly declares the new component identity. Its additive
migration cannot delete/reuse identities, omit new graph nodes, hide new edges,
or waive version bumps of affected consumers. The integrated iPad inset fix also
versions `@cowboy/app-shell` to 1.1.1; its source change had not yet been recorded
in main's component matrix. Other previous components and all seven Plugin
sources/bindings remain identical. No signed Plugin or public
legacy SDK bytes are rewritten; this is not an npm/Catalog publication or a
Machine installation.

The Rust embedded-registry reader also validates the explicit addition while
retaining closed decoding and historical schema-two support. The live Controller
continues using its own embedded matrix and unchanged Plugin pins; Web needs no
Controller/Machine restart or remote metadata/schema cutover for this addition.

## One interactive view, one core owner

`web/src/providerUiOwner.ts` interprets the independently validated immutable UI
snapshot. The owner is local to the exact manifest, slot and core target /
execution-release binding. A refreshed equal manifest retains its owner; a
changed binding gets a new one. Machine panels are keyed by Machine ID, not
display name. React creates owners at layout-effect commit, disposes them on
retirement, and observes their snapshots with `useSyncExternalStore`.
Development StrictMode cleanup/replay creates a fresh usable owner.

Admission resolves all reducers, including an effect rule after a pure rule.
Only the owner's actual surface buttons can emit. It rechecks ancestor
visibility, enabled conditions, the latest committed blocked-capability set and
the presence of a core callback/target. It reserves the task and busy state
synchronously before observers or host callbacks can reenter. One effect can be
pending per interactive view; other views/Machines remain independent. This is
not a global install mutex: the Controller/Machine's authorization and lifecycle
fences remain authoritative.

Before dispatch, the current executor requires an empty request schema, success
`{}`, failure `{ detail: string }`, and effect-free completion reducers.
Unsupported profiles display a compatibility warning and cannot issue a host
request. A declared request is never silently ignored, and a
completion-triggered effect is never silently dropped after an irreversible
operation.

Settlement means the trusted callback completed. In particular, opening a
Service sign-in flow is not proof of authentication, and receiving an uninstall
plan is not an uninstall receipt. Raw backend exception detail remains in core
management; Plugin state gets a fixed failure message. The callback also
receives a core observation fence: retired views cannot publish late errors or
open a late uninstall-plan dialog. Backend operations already submitted still
finish.

Disposal seals new emissions synchronously, releases subscriptions and waits for
admitted callbacks to settle. A stuck callback stays draining. Late success,
failure and cleanup cannot mutate a new owner's state or clear its busy flag.
There is no automatic cancellation, retry, compensation or claim that closing a
view can revert an external effect. No owner is acquired for pure cards,
information or activity rendering; these stay ordinary lightweight components.

## Verification and remaining boundaries

`just check-compact` includes strict compile-negative authoring tests, runtime
owner races, SDK builds and executable-profile checks for all six existing Agent
Plugins, plus additive-migration rejection tests. The optional
`just provider-ui-browser-conformance /nix/store/…/bin/firefox` uses the pinned
test browser, fresh profile and a private loopback-only network. It exercises
real React/MUI StrictMode, double click, equal Catalog refresh, target/release
replacement, capability changes and unmount/remount with fake host effects. The
shared IDB browser suite remains independently selectable and checked.

This slice does not retire the old public mixed SDK/native ABI, change
production CoreSecurity policy, alter existing auth/install wire contracts or
accept a real native/physical device. The subsequent
[management-dialog slice](provider-management-ownership.md) owns the core
sign-in/confirmation observations and explicit requests. Neither slice supplies
durable Service/Machine execution, remote cancellation on unmount or distributed
compensation. Those remain separate P1/P2+ acceptance boundaries. Web-only
activation must preserve Controller, Machine and active worker processes.
