# Cowboy Provider UI SDK

The SDK defines Cowboy Provider UI schemas v1-v2 and Provider SDK 2.2 as closed
TypeScript unions. It provides runtime validation, typed state transitions,
text resolution, boolean-expression evaluation, responsive stack wrapping,
bounded vector gradients, typed activity strategies, component-command links,
and session-sidecar URL links. Provider packages contain data only: the Cowboy
host renders the IR, owns every privileged effect, and launches the exact signed
runtime graph without evaluating Provider code or shell templates.

UI schema v2 adds the closed `activity` node and the `glyph_cycle`,
`terminal_prompt`, `asset_signal`, `asset_pulse`, and `progress_ring` indicator
strategies. Labels are typed text or bounded phrase cycles with a closed
`none`/`fade`/`shimmer` effect. Cowboy owns animation mechanics, reduced-motion
behavior, accessibility, theme integration, and responsive layout; a Provider
supplies only validated parameters and referenced assets. Schema v1 remains
readable and receives Cowboy's compact compatibility presentation.

The canonical cross-language schema and package validator are in
`crates/cowboy-provider-sdk`; this package is the browser/component-library
view of the same contract. `just provider-check` independently builds all six
first-party packages, then parses every resulting manifest through this
TypeScript validator so Rust/TypeScript contract drift fails before release.
