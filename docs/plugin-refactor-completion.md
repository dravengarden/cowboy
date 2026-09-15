# Plugin refactor completion ledger

Reviewed against the 2026-09-09 [target architecture](plugin-spatiotemporal-design.md)
and current code on 2026-09-15. This is the current exit checklist, not a list of
all historical slices. Passing a test, publishing a reader, enabling a policy
and accepting a production effect are different milestones.

## Implemented and accepted in automated gates

- One immutable signed Plugin lifecycle and dependency-closure release matrix;
  six independent Agent sources plus Zed, with no installable internal runtimes.
- Core-owned communication, installation, security/native/Web host boundaries;
  isolated Provider accounts, exact session/workspace generations and sidecars.
- [Controller-owned Catalog observation](unattended-release-adoption.md) uses
  bounded event settling, public trust-directory watches and read-only metadata
  fallback for missing/replaced roots. Unchanged hints skip host rebuilds;
  shutdown stops new attempts and drains admitted refreshes before storage
  teardown. Signed-reader and lifecycle tests retain exact release leases on
  rejected candidates and distinguish a committed Plugin snapshot from a failed
  Provider projection. This is not unattended Machine installation, a new
  deployment credential, physical rollback or post-effect recovery. Its
  [Controller release](releases/plugin-catalog-observer-2026-09-15.md) passed both
  complete gates and identical 69-release reads by actual candidate, predecessor
  and cold Controllers; it is published and activated with all 16 observed
  workers retained. Unattended installation remains explicitly deferred.
- Generated closed composition wire types and bounded, read-only structural
  checkers in Rust and TypeScript; combined ownership/capability cycle checks
  and an 86-case real-CLI differential gate compare complete link reports. This
  is not verified live resolution or a grant.
- [Finite live telemetry resolution](resolved-telemetry-ports.md) links exact
  verified contracts and original per-installation observation leases to the
  existing binding/export executors. An observed invalid installation cannot
  revive an old operation by returning to the same tuple. This is not general
  graph resolution, a state-dataset lease or new execution authority. Its
  [Controller release](releases/resolved-telemetry-ports-2026-09-14.md) passed
  all 45 connected flows and nine Victoria pairs and was activated without
  restarting the resident Machine, workers or Victoria processes.
- [Core verified release observations](plugin-release-leases.md) preserve the
  original exact Catalog lifetime across installation, uninstall and telemetry
  awaits. Accepted removal/re-addition or publisher-envelope replacement cannot
  revive old confirmation; rejected candidates and unrelated releases preserve
  continuity. This extends the finite trust boundary, not graph/state authority.
  Its [Controller release](releases/plugin-release-leases-2026-09-15.md) passed
  807 immutable role checks, including actual install/uninstall preflight ABA,
  and is activated with Machine and worker identities retained. This does not
  add generic graph execution or independent post-effect recovery.
- [Service-bound execution Sites](plugin-service-sites.md) require a core-loaded
  identity owner and compare the exact Service/Machine pair in finite resolution,
  preflight and both final outgoing paths. All twelve scoped command variants,
  including historical queries, share the check; generic send/RPC entrypoints
  cannot bypass it. The [Controller release](releases/plugin-service-sites-2026-09-15.md)
  passed all 807 immutable role checks and is active with Machine and all 15
  observed workers retained. This does not resolve Workspace/Session scopes,
  grant state ownership or authorize a graph.
- [Finite code-read scopes](plugin-code-read-scopes.md) carry exact process-local
  Session incarnations through Zed requests/replies and all eleven buffered
  filesystem/Git HTTP readers, including cached, conditional and error replies.
  Cwd ABA and delete/recreate cannot revive them. Advertised Workspace snapshots
  include Service/Machine/workspace/path. Per-entry diff cursor identities
  prevent equal-content aliases and unsafe UTF-8 slicing. Core Code requests
  serialize the existing closed Rust operations with unchanged wire shapes.
  This does not fence all Code effects, supply continuous Workspace or filesystem
  identity, grant state authority or accept a native generation. The
  [initial diff/Zed release](releases/plugin-code-read-scopes-2026-09-15.md)
  passed the complete gate and retained its 15 observed workers. The expanded
  [buffered-reader Controller release](releases/plugin-buffered-code-reads-2026-09-15.md)
  adds six response-boundary tests and fourteen wire vectors, passed the complete
  gate, and is published and activated with all 16 observed workers retained.
  File continuations now bind the same original context and exact requested
  path in a bounded Controller registry before forwarding a native cursor.
  Representation ETags include continuation identity; expiry cannot return a
  stale cursor via `304`. Local and cached readers repair UTF-8 page/view
  boundaries and incomplete EOF. Independent Code adapter tests are part of the
  complete gate; source coverage is not a running remote adapter upgrade.
  The [file-page Controller release](releases/plugin-file-page-scopes-2026-09-15.md)
  passed both complete gates and is published and activated with 16 workers
  retained; its independently built core adapter passed seven actual disposable
  process cases without upgrading a resident Machine.
- Zed Session operations now retain their original authenticated Machine
  connection or connected local Unix peer through worktree readiness and buffer
  open. Same-epoch reconnect and pathname replacement cannot redirect the
  sequence; cancellation/failure ends it without retry. Local writes/reads and
  message bytes are bounded, and non-ready worktrees cannot open buffers.
  Sixteen new source tests cover these boundaries. This is not native-generation
  ownership, cross-request buffer release, compensation or post-effect recovery;
  see [the precise remaining lease gap](plugin-code-read-scopes.md#zed-operation-connection-lifetime).
  The [Controller release](releases/plugin-zed-operation-scopes-2026-09-15.md)
  passed the complete gate and matching 72-release reads by actual candidate,
  predecessor and cold binaries. It is published and activated with all 12
  observed workers, Machine and Victoria retained in its deployment window.
- [Prepared native buffer references](plugin-native-buffer-leases.md) implement
  effect-free preparation, non-recycled native identities, exact Machine runtime
  retention, path-free release/query, bounded capacity and cancellation-safe
  cleanup. Open/ambiguous effects do not expire or replay. This is a native/Machine
  candidate, not the ordinary Controller/Web resource owner or a production
  native-generation acceptance claim. Capability-floor negotiation, client
  integration and separate Machine/Code Plugin activation remain required.
- Typed Provider authoring, owned UI effects, state-store/resource scopes,
  subscription/task drain and explicit IDB connection/transaction ownership.
- [Atomic IDB outbox deltas](atomic-idb-outboxes.md) preserve updated peers'
  pending mutations and confirmations in one transaction, with explicit load
  handoff and strict durable-send failure. The v1 data format is unchanged;
  that release alone does not fence pre-upgrade blind writers or grant general
  dataset authority.
  The [Web release](releases/atomic-idb-outboxes-2026-09-14.md) passed both
  real-browser suites and activation checks without restarting live workers.
- [Core product browser datasets](product-sync-datasets.md) now implement
  immutable Service/principal binding, closed Service/Session keys, exact IDB v2
  writer fencing, transaction-lifetime connections and bounded read-only legacy
  export. Source gates include 16 real-browser outbox cases and eight connection
  lifetime cases. The historical [Controller bridge](releases/product-datasets-and-lifecycle-2026-09-15.md)
  is superseded by the [activated Web, bound Controller and compatible cold floor](releases/dataset-bound-maintenance-2026-09-15.md).
  Actual active/next-recovery/cold Controllers all pass bound-mode acceptance.
  This closes the finite deployment sequence, not general exclusive state
  leases, physical-device acceptance or adoption of unowned browser records.
- Service uninstall journal, Machine uninstall receipts, installation-incarnation
  CAS, execution leases, continuous Operator checks, read-only recovery and a
  separately confirmed abort of a proven pre-effect Service interruption.
- [Durable Service installation attempts](plugin-install-journal.md), including
  exact operation identity, original confirmation/connection, prior-committed
  effect phases, install/uninstall claim exclusion and restart fences; closed,
  reloadable history is shared by the two installation clients. The
  [historical schema-one writer milestone](releases/plugin-install-journal-2026-09-14.md)
  is now superseded by the [accepted Service/Machine receipt reader floor](releases/plugin-install-receipt-readers-2026-09-14.md)
  and [activated protocol-19 writer](releases/plugin-install-writers-2026-09-14.md).
  Two complete 45-case connected installation runs and another 240 actual-role
  reader checks passed. No production Plugin installation or native-generation
  swap is claimed.
- Core local-security ownership implementation and crash-recoverable adoption,
  without moving credentials or changing historical SQL bytes.
- [Unified core lifecycle history](plugin-lifecycle-history.md) projects the
  actual install/uninstall journals and independently confirmed pre-effect
  resolution with domain-disjoint IDs, bounded no-store reads and no Machine
  RPC or effect path. Both installation clients share the view; source tests
  include the exact Rust/Web fixture and six real React/browser lifecycle cases.
  Its actual immutable Controller HTTP acceptance and Controller activation
  [passed](releases/product-datasets-and-lifecycle-2026-09-15.md); the shared Web
  consumer is now [activated with the dataset release](releases/dataset-bound-maintenance-2026-09-15.md).
- Finite Victoria binding/revoke/restore, independently authorized recovery and
  settlement, bounded managed OTLP export, standing policy and private admission.
  Actual immutable role gates cover 96 reader, 294 writer, 78 startup and 45
  connected-flow cases; [real Victoria acceptance](releases/telemetry-victoria-conformance-2026-09-14.md)
  adds nine database-backed role pairs and reopen/no-replay checks.

## Code work still required

| Exit | Remaining implementation | Required evidence |
| --- | --- | --- |
| P0 / typed resolution | Extend verified release observations, finite Service/Machine Site checks, telemetry resolution and code-read observations to applicable graph contracts, continuous Machine-owned Workspace/Session/security-domain identity, state leases and policy; link exact resolved results to finite domain executors | General graph/site/state-lease vectors beyond accepted-Catalog, finite Site, code-reader and telemetry installation fences and shared structural link vectors; no serialized authorization |
| P3 / state compatibility | General state-dataset identity and reader/writer coexistence beyond the finite security, telemetry and now-deployed browser namespaces | Actual old/new readers and writers, exclusive fenced ownership, principal changes, crash/reopen, version-change and independent workspace/generation coexistence |
| P4 / capability acceptance | The core [connected installation writer](releases/plugin-install-writers-2026-09-14.md) is active and its Victoria installation/reinstall/fault matrix is accepted; extend actual immutable process acceptance to Agent authentication projection and code/native integration | Each supported Plugin lifecycle, cancellation/crash at its additional capability boundaries, same-ID deduplication and changed-input refusal; no native restoration inferred from a telemetry fixture |
| P4 / recovery | Independently authorized post-effect Plugin restoration and verified exact native-worker recovery; bounded evidence archival that retains unresolved references | Installation CAS, fresh recovery purpose/budget, partial outcomes, no restored-turn claim, no credential rollback or userdata deletion |
| P4 / diagnostics | Extend the deployed unified install/uninstall/resolution Web consumer as additional finite domains and post-effect recovery exist | Reloaded UI reads durable status without replay or fabricated completion; recoveries remain explicitly separate and independent observations never claim an atomic cross-domain snapshot |

The [install continuation repair](plugin-install-continuation.md) closes live
HTTP cancellation, stale authority/connection and unsafe fence-release gaps. It
is [published and activated on Controller/Web](releases/plugin-install-continuation-2026-09-14.md),
and the subsequent Service installation journal adds restart protection.
The Machine/Service receipt writer now has its own connected acceptance and
activation evidence; none of these closes the independent recovery row above. Local disposal does
not implement cross-site rollback, and a read-only composition is not an
authorized generic DAG. Do not add a parallel Plugin lifecycle or expose an
unconstrained workflow executor to hide these gaps.

## Production and supported-client acceptance still required

- **Supported browser and retained-worker acceptance:** dataset-aware Web, bound
  Controller, compatible cold recovery and independently authorized Machine
  maintenance are [activated](releases/dataset-bound-maintenance-2026-09-15.md).
  The TLS floor is active in Controller and resident Machine, but retained worker
  generations were not force-replaced; their native-generation acceptance and
  actual supported-device reload/storage checks remain. One independent client
  context reset crossed the observation window and is not native-continuity
  acceptance. Old browser records remain unowned and retained, never auto-replayed.
- **Core security handoff:** actual Password, device-authorization completion and
  Passkey tests; supported native origin/gesture tests; then the owning committed
  host policy, stopped-Controller adoption and complete compatible recovery
  configuration. Removing generated local-auth pins or authority markers is not
  acceptance. Retire old public SDK/native entries only after these checks.
- **Managed Victoria cutover:** actual Operator confirmation, both independently
  owned writer policies, complete standing export policy and private destination
  authentication/TLS, followed by real ingestion/query and controlled failure /
  restart acceptance. The first managed intent permanently fences legacy export,
  including rejected/aborted attempts; account for that interval explicitly.
- **Session/generation acceptance:** verify retained and upgraded native sessions
  and multiple workspace/runtime generations on their supported platforms.
  Preserved worker PIDs in a bounded deployment window do not prove an actual
  generation swap or native resume. No forced rebind, login or worker restart is
  authorized merely by a Controller/Web release.
- **Remote core Code adapter:** the shared UTF-8 paging repair is source-tested
  in the independent feature graph and its exact immutable artifact passed seven
  disposable process cases, but Controller activation updates
  only colocated reads and Controller-owned cursor bindings. Accept and activate
  the exact remote adapter through its separately authorized Machine release;
  do not infer that upgrade from the Unix socket fixture or retained Machine PID.

These checks require real account/device participation and, for the Machine or
host-policy change, the separate maintenance boundary. Synthetic fixture keys,
isolated databases and an agent's filesystem access cannot substitute for them.
The physical iPhone pasted-image caret issue in `web/src/mdlive/PITFALLS.md` #69
also remains unsolved; it is not a Plugin-refactor completion claim.

## Completion rule

Do not report the whole refactor complete until the code exits and the production
and supported-client exits above have recorded evidence. Preserve the distinction
between implemented, verified, published, activated and actually accepted.
