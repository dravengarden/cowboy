# Machine durable-state diagnostics — Hawk, October 6

A read-only, content-free report of a Machine's durable datasets, served by the
Controller and read with `cowboy operator durable-state --machine <id>`. Design, schema
and limits are in [the diagnostics note](../machine-durable-state-diagnostics.md).

## Releases

All three are ordinary component releases from `main`, none of which touches the retained
worker interface:

| Lane | Source | Result |
| --- | --- | --- |
| Machine (resident, writer host release) | `5d4d0c884f041a23596cdf6f8dc438a976cc1b2b`, `/nix/store/lxffj5gr…-cowboy-machine-writer-host-release` | receipt `5d4d0c884f04`, `2026-10-06T13:40:54Z`–`13:41:02Z`, succeeded, committed; worker generation `worker-d62183a8…` unchanged |
| Controller (route) | `5d4d0c884f04…`, `/nix/store/dcxz2p5y…` | `13:47:54Z`–`13:48:06Z`, succeeded |
| Controller (route and Operator subcommand) | `5553d276bf8879dc3ef273257c8b1981f5d71f9b`, `/nix/store/fdslw9vi…` | recorded `14:12:04Z`, succeeded |

The first Controller release carried the route only; it needs a product session or the
Operator CLI, so it was not usable from the host until the second added the subcommand. A
first dispatch of the second was stopped by a transient GitHub fetch failure before any
effect and repeated. A new source file in a Machine release has to be listed in the explicit
Machine source set in `flake.nix` (the plain `cargo check` does not show this; the Nix build
failed with `file not found for module durable_state` until it was).

## Validation

Rustfmt, both Clippy configurations, 630 standalone and 1974 all-features tests. New tests
cover the closed schema (unknown fields, another schema, hostile and negative counts,
partial reports), the Machine request being refused unless it is an empty object, the view
being empty until a broker attaches, reporting counts and writer flags without naming a
Session, lineage or path, reader-only builds saying so, and never keeping a broker alive,
the route classification, and the subcommand accepting exactly one Machine. Accepting any
request payload fails the Machine test. Native production conformance, active against new
writer with same-generation reader-only releases: **45 groups accepted**
([receipt](../experiments/durable-state-diagnostics-native-conformance-2026-10-06.json)); it
does not exercise the new adapter.

## Production observation

`cowboy operator durable-state --machine hawk` returned HTTP 200 with
`deletionJournal {writerEnabled: true, deletedSessions: 7}`, `sessionIncarnations
{writerEnabled: true, lineages: 7}` and `cleanupContinuations {pending: 0}`. The two counts
matched the committed files exactly (7 and 7). An unknown Machine returned 404. Around the
Machine activation all 27 worker/keeper units kept their IDs, PIDs and states and the
Controller PID did not change; around the first Controller activation the Machine PID and all
27 units were unchanged and only the Controller PID moved. An unauthenticated HTTP request to
the route got `426 HTTPS is required`, which is weak evidence that it is not public, since
that check precedes authentication; the route classification test is the stronger evidence.

## Limits

No Web panel exists. The report is counts only, read dataset by dataset, and cannot show a
lineage change or the root-owned floors. It reflects the Machine's open namespaces, not an
audit of the files, and was observed once. The Machine release's native conformance does not
cover the new adapter. Reading it requires the Operator delegation that already existed on
this host.
