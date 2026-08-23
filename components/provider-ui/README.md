# Cowboy Provider UI SDK

The SDK defines Cowboy Provider UI schemas v1-v2, host integration schemas
v1-v2, and Provider SDK 2.4 as closed
TypeScript unions. It provides runtime validation, typed state transitions,
text resolution, boolean-expression evaluation, responsive stack wrapping,
bounded vector gradients, typed activity strategies, component-command links,
session-sidecar URL links, and typed Transcript presentation. Provider packages
contain data only: the Cowboy host renders the IR, owns every privileged effect,
and launches the exact signed runtime graph without evaluating Provider code or
shell templates.

UI schema v2 adds the closed `activity` node and the `glyph_cycle`,
`terminal_prompt`, `asset_signal`, `asset_pulse`, and `progress_ring` indicator
strategies. Labels are typed text or bounded phrase cycles with a closed
`none`/`fade`/`shimmer` effect. Cowboy owns animation mechanics, reduced-motion
behavior, accessibility, theme integration, and responsive layout; a Provider
supplies only validated parameters and referenced assets. Schema v1 remains
readable and receives Cowboy's compact compatibility presentation.

Host integration schema v2 adds a bounded Transcript thought contract. A
Provider selects `timeline`, `workcell`, `signal`, or `terminal`, plus a closed
density, optional active label, and current-step surface token. Cowboy owns the
marker geometry, typography, animation, reduced-motion behavior, theme colors,
and accessibility. Host schema v1 cannot declare these fields, and unknown
variants or tokens fail validation before rendering.

The sanitized authentication projection exposes a closed `account` or
`api_key` presentation derived from typed auth methods. Cowboy uses it for
status, actions, and modal language while retaining card geometry, responsive
layout, credential handling, and effects in the host component library.

The canonical cross-language schema and package validator are in
`components/provider-sdk`; this package is the browser/component-library
view of the same contract. `just provider-check` independently builds all six
first-party packages, then parses every resulting manifest through this
TypeScript validator so Rust/TypeScript contract drift fails before release.
