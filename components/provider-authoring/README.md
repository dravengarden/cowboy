# @cowboy/provider-authoring

Pure, data-only authoring for the existing Provider UI IR. This package has no
React, native, registration, network, installer or resource-owner API. It peers
with exactly `@cowboy/provider-ui` 3.1.12; it does not replace its runtime
verifier or change any signed package schema.

Wrap a literal `{ logic, ui }` in `defineProviderUiContract(...)`, then spread
the result into the full Provider manifest and run the normal SDK validator and
packager. The helper returns the same data, without changing serialization. Keep
declarations literal: widened `ProviderUiContract` / parsed JSON is rejected by
the helper's types, not granted authoring proof.

State initial values, message payloads (including excess keys), reducer targets
and sources, completion-message IDs, UI state/asset references, conditions and
surface effect ownership are linked at compile time. `ProviderStateOf<C>` and
`ProviderMessageOf<C>` expose the inferred state and discriminated messages for
author tooling. TypeScript structural message types alone do not enforce exact
object keys; the helper checks each actual button emission separately.

Safe-integer bounds, duplicate IDs, total sizes, schema versions and
fingerprints are still runtime checks. Casts/`any` are not a security boundary.
Downloaded artifacts must pass independent validation and signature
verification. An effect declaration is data, not permission: the current core UI
executor additionally requires its supported request/completion profile before
dispatch. This is a Provider UI authoring slice, not the future general
distributed Plugin DAG DSL.
