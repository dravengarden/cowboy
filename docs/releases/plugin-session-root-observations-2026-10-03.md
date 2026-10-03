# Machine-owned Session read roots — October 3

A remote Session read previously bound its logical Session and original Machine
connection but had no observation of the directory behind its cwd. Replacing
that directory with another object containing identical bytes could therefore
return an old conditional 304 or reuse a page continuation.

The [protocol-25 contract](../plugin-session-root-observations.md) adds a private
Machine-owned directory observation after original product authorization and
before cache/cursor lookup. The incarnation binds the original read scope;
Machine checks surround Code dispatch and complete Controller responses. Root
replacement suppresses the whole stale response with 410/no-store/no ETag, old
page continuations expire, and fresh reads may observe the replacement object.
Native ownership, Session lifecycle, security domains and state leases remain
separate. Older peers retain an explicitly legacy remote read path.

## Implementation and source checks

- Reader source: `47faead6`; published integrated source `7e3c2cd5`.
- Directory-only open follow-up: `645578ee`; final Machine source `2b54b598`.
  `O_DIRECTORY` rejects a FIFO before an open can block a runtime thread.
- Pinned complete gate passes formatting, lint, dependencies, composition,
  feature/type checks, 1,734 initial Rust tests, 432 initial Machine tests,
  31 core-adapter tests, 126 private-adapter tests, 1,993 initial Web tests,
  22 PostgreSQL tests and release builds.
- Final reader source passes all 1,735 main Rust tests and actual integration
  binaries. Final directory follow-up passes all 434 standalone Machine tests
  and full all-target/all-feature lint. Focused tests cover original-connection
  replies, cache identity, replacement/restoration, bounded eviction, accepted
  configuration ABA, original symlink targets and special-file refusal.
- Integrated Web lint/types and all 1,999 tests pass before the reader adoption.
  Later independently integrated external-sign-in and WireGuard research changes
  do not change the affected backend implementation.

One overlapping Cargo rebuild caused the self-spawning process-cleanup test to
fail at spawn. Its complete serial rerun passes without changing that test or
product timeouts. A host-only `clippy --tests` attempt reaches the known
unguarded ACP-worker integration feature boundary; the required standalone
Machine lib tests and all-feature lint pass.

## Real immutable negative and positive

The [negative](../experiments/plugin-session-root-negative-2026-10-03.json)
supplies the preceding active Controller (`b06c3153`) and Machine (`18dc97a1`).
It negotiates protocol 24, successfully installs Code, then returns incorrect
HTTP **304** after the real held Session reply's directory is replaced. It
fails at `machine_owned_session_root_identity`; cleanup succeeds.

The final [connected receipt](../experiments/plugin-session-root-connected-2026-10-03.json)
supplies exactly the active Controller below and the final Machine below.
All **38 v17 checks** pass in 294.78 seconds; five connections/configurations
negotiate protocol 25 and cleanup succeeds. The new checks discard the held
conditional reply after real root replacement, refuse its old cursor without
Code I/O, and permit a fresh read. Existing installation, authorization, native
ownership, reconnect, Controller restart and seven real 40-second reply-loss
checks remain intact. Fixture browser clients use real device signatures and
boot challenges with the isolated same-host HTTPS-proxy header boundary;
this does not establish browser/device or external TLS acceptance.

An earlier reader candidate also passes all 38 checks in 294.71 seconds.
The subsequent Web-only main merge changes the release provenance envelope;
[entry-by-entry equivalence](../experiments/plugin-session-root-equivalence-2026-10-03.json)
permits only the release's own path and source revision normalization. The
final follow-up's complete connected run uses the exact final supplied artifacts.

## Hawk activation

- Controller source `7e3c2cd5848bfc814e2999bf555d89d844032551`:
  `/nix/store/fqrvkfszwszsqvfr2qsibv59s4jpbrlf-cowboy-controller-release`.
  [Receipt](../experiments/plugin-session-root-controller-activation-2026-10-03.json):
  transaction `1791001934470292057-7e3c2cd5848b`, succeeded/committed/published.
- Final Machine source `2b54b598bd9456c1d8a4dcbd25658cb7fb53599d`:
  `/nix/store/izrpw18v0wgj494mlz84d09y53n9kzwn-cowboy-machine-release`.
  [Receipt](../experiments/plugin-session-root-machine-activation-2026-10-03.json):
  transaction `1791003893410091625-2b54b598bd94`, succeeded/committed/published,
  maintenance true, recorded `2026-10-03T05:05:02.46824107Z`.
- Worker generation stays `worker-eed1d8105af00846771d`. The initial Machine
  adoption is retained in its [separate receipt](../experiments/plugin-session-root-initial-machine-activation-2026-10-03.json).

[Continuity](../experiments/plugin-session-root-continuity-2026-10-03.json)
records every original unit before and after each component activation. All
**14 ACP workers and two execution keepers** retain PID/active/running state
across each activation. One worker PID changes between the two release windows,
already present in the follow-up before snapshot; the receipt records that
interim change separately rather than claiming uninterrupted PID continuity.
The final Machine is PID 2377011; Controller PID 1998362 is unchanged by the
Machine follow-up. Machine authentication logs confirm protocol 25 at
`2026-10-03T05:05:02.064856Z`. Health and version return 200; index and service
worker return 200 with no-store. A separate concurrent Web deployment changes
the SPA hash from `1cda721d...` to `f6d1eafd...`; its own receipt is included,
so this record does not attribute that Web activation to these backend releases.
