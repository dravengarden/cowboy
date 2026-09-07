# Retired pre-cutover Catalog reader bridge

The deployed legacy Controller floor was `c0911dd012bad4191f2936491c3879346cffbd25`.
Its three branch-only commits backported read-only Catalog inspection, rejection
of post-host-cutover authority, and isolation of unsupported release / nested Code
formats. They changed only `src/cli.rs`, `src/server.rs`, `src/plugin_catalog.rs`
and this historical documentation; they contained no unrelated product changes.

The full Plugin successor already implements read-only inspection and bounded,
fail-closed release parsing, and understands host-bearing release schema 2 and
Code payload schema 2. Its exact host policy, durable authority and storage
migration checks replace the bridge's unconditional post-cutover rejection.
The integration therefore retains the reviewed successor implementation instead
of reinstating the legacy schema-1 exclusions or host-startup prohibition.
The ancestry merge records the active deployment floor for the machine-owned
activator; it is not a new claim of historical Provider coexistence acceptance.

The 2026-09-07 rollout was explicitly authorized to upgrade all versions. Keep
historical Catalog bytes and failure receipts intact, but do not gate this rollout
on running retired Provider generations. New release integrity, current runtime
acceptance, exact host policy and data preservation remain required.

The legacy bridge cannot recover activated Plugin hosts/storage. After the first
host authority is recorded, recovery must use a host-capable successor with its
exact policy; never delete authority markers or applied migration records to
force an old Controller to start.
