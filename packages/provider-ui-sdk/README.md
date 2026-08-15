# Cowboy Provider UI SDK

The SDK defines Cowboy Provider UI schema v1 and Provider SDK 2.1 as closed
TypeScript unions. It provides runtime validation, typed state transitions,
text resolution, boolean-expression evaluation, component-command links, and
session-sidecar URL links. Provider packages contain data only: the Cowboy host
renders the IR, owns every privileged effect, and launches the exact signed
runtime graph without evaluating Provider code or shell templates.

The canonical cross-language schema and package validator are in
`crates/cowboy-provider-sdk`; this package is the browser/component-library
view of the same contract. `just provider-check` independently builds all six
first-party packages, then parses every resulting manifest through this
TypeScript validator so Rust/TypeScript contract drift fails before release.
