# Core Plugin lifecycle history

`GET /api/machines/{machine}/plugins/{plugin}/lifecycle-history` is an
Operator-only, no-store core diagnostic. It projects saved installation and
uninstallation attempts plus the latter's independently confirmed pre-effect
resolution. It does not contact the Machine, refresh inventory, execute a
Plugin, create a plan, repeat a confirmation or clear a lifecycle fence.

The exact `dravengarden.cowboy.plugin-lifecycle-history/v1` envelope is bounded
to 128 KiB and a five-second Service read. Each durable journal contributes
its latest 32 attempts by creation time; display order uses their recorded
update times. This is a bounded window, not a complete archive, causal DAG
order, current installation status or an atomic cross-domain snapshot.
`observation=independent_durable_reads` and `execution_authorized=false` are
explicit. Machine and Plugin must match the requested target; the Service is
selected by the actual server, never by user-supplied history data.

Entries use a closed `install | uninstall` discriminant. Identity is `(kind,
operation_id)`: separate durable journals can legitimately reuse the same ID.
Install entries reuse the existing schema-one/two evidence and strict Machine
receipt projection. Uninstall entries disclose phases, closed problems/cause,
release identity, timestamps and approved session **count**, not session IDs.
Actors, credentials, destination policy, full Machine steps, private evidence
digests and expiring permits are not serialized into this view.

A saved resolution is a separate identity nested under its original uninstall,
never a replacement operation or grant to replay it. The Service verifies the
complete saved resolution against its original operation before projecting
`abort_before_effects`. An interleaved read that observes an incompatible
operation/resolution pair fails closed; refreshing is a new read, not a repair.
The receipt explicitly reports no Plugin/session mutation or worker restoration.
Legacy compensation phases remain labeled historical; they do not prove a
restored native turn or independent post-effect recovery.

The Product and Admin installation clients share `PluginLifecycleHistory`.
Collapsed views do nothing. Opening or refreshing performs one bounded GET,
never POST, Machine receipt RPC, automatic retry or cached-confirmation replay.
Each request is owned by its mounted target and cancelled at target replacement,
unmount or synchronous product-session end. Previous-target evidence cannot
paint while effect cleanup is pending. Failed/incompatible reads show
unavailability, not an empty history. The eight-second client deadline and
128-KiB streaming decoder bound a misbehaving server.

## Evidence and remaining scope

Rust projection and Web decoding share
[`plugin-lifecycle-history.json`](../tests/fixtures/plugin-lifecycle-history.json).
Tests exercise domain-ID collisions, exact typed receipts, independent resolution,
target scoping, repeated read-only Store access and closed size/field/phase
boundaries. The optional `just plugin-lifecycle-browser-conformance <browser>`
gate uses real React/MUI under StrictMode with synthetic deferred HTTP replies,
a fresh pinned browser profile and private loopback. Its six cases cover open,
refresh, changed target, failed reads, logout and unmount. No live login or
production operation participates.

The immutable Controller handshake gate also accepts an optional three-element
`lifecycle_history` boolean array, in active/recovery/cold order. For each true
role it seeds only disposable pre-effect journals, opens the actual Controller
twice, checks anonymous/Viewer refusal and Operator no-store reads, and verifies
that three repeated HTTP reads leave the exact saved attempts and resolution
unchanged. False is **not checked**, not an accepted history reader. Actual
artifact acceptance and activation must be recorded separately.

The existing installation/uninstallation APIs retain their bodies and now
explicitly return no-store for successful history reads. This
replaces only the two core history consumers, not Plugin packages or the Catalog.
Telemetry binding/recovery already has separate typed surfaces and is not
silently folded into a supposedly atomic lifecycle transaction. General graph
diagnostics, post-effect restoration, archival and supported-native acceptance
remain separate [completion exits](plugin-refactor-completion.md).

During the 2026-09-15 gate, newly published
[GHSA-2mjx-qc3c-rqvc](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc)
rejected the retained Cowboy Rustls 0.23.40 dependency. The minimum and lock now
select the official patched 0.23.45 and its WebPKI 0.103.15 dependency; no advisory
exception or TLS policy relaxation was added. The Nix vendor staging checksum
was recomputed and verified with the owned builder. This changes the core
Machine/worker build generation too: passing source gates or activating a
Controller does **not** update retained Machine/worker processes, native clients
or independently packaged Agent dependencies. Their maintenance/acceptance
boundaries remain explicit.
