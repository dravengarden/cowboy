# Plugin refactor completion ledger

Reviewed against the 2026-09-09 [target architecture](plugin-spatiotemporal-design.md)
and current code on 2026-09-16. This is the current exit checklist, not a list of
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
  native-generation acceptance claim. Its [candidate evidence](releases/plugin-native-buffer-leases-2026-09-15.md)
  includes 29 new tests, complete gates, immutable builds and real temporary
  signed Zed install/uninstall/path-removal drain. Production core-probe acceptance, client
  integration and separate Machine/Code Plugin activation remain required.
- [Controller buffer ownership](plugin-controller-buffer-owners.md) adds a
  product-only, bounded prepare/open/query/release API over those references.
  It retains the original Session incarnation and authenticated Machine
  connection; observer cancellation cannot cancel an admitted mutation, and
  ambiguous effects never replay or expire. Cleanup requires the original
  resource user but not a still-existing Session/path. Ordinary Review language
  reads and buffer calls remain legacy; native-generation rollout, abandoned
  browser cleanup, restart restoration and independent recovery remain open.
  Its [Controller candidate](releases/plugin-controller-buffer-owners-2026-09-15.md)
  passed the complete gate (1,317 all-feature Rust tests) and immutable build.
  Its original six-publication prerequisite is now closed by
  [independent signed Agent releases](releases/agent-publication-2026-09-15.md):
  Linux old/new worker gates, actual Mac probes, four immutable reader roles,
  all 24 public artifact URLs and automatic 78-release Catalog adoption passed.
  The separately activated Controller `0fded719` includes this implementation;
  the publication task retained all 13 observed workers and changed no installed
  Plugin or component. Authenticated Catalog API/native acceptance remains
  unchecked. The later [Claude Code `3.1.23` publication](releases/claude-stream-recovery-2026-09-15.md)
  now has its own Linux/Mac and 79-release reader/adoption evidence; neither
  publication installs it into retained sessions.
  The subsequent [Claude Code `3.1.24` publication](releases/claude-plan-usage-2026-09-16.md)
  is also integrated; its native quota collector still requires separate Plugin
  activation. The owned-buffer release below accepts the resulting 80-release
  Catalog without changing an installed Plugin.
- [Owned buffer observations](plugin-owned-buffer-reads.md) add closed diagnostics
  and symbol reads borrowing the original core/native owner, with fresh original
  credential, Session and connection checks and bounded typed result validation.
  Reads never re-resolve the source path or change effect evidence. Zed `1.4.0`
  consumes actual diagnostic buffer updates, acknowledges their transport and
  propagates native language transport failures rather than returning false empty
  success. Observed edits/reloads invalidate its bounded base coordinates.
  Its [Controller release](releases/plugin-owned-buffer-reads-2026-09-16.md) passed
  the complete gate and actual static-Zed conformance and is active with all 13
  observed workers retained; the native Plugin remains an uninstalled candidate.
  Hover/navigation still need actual content/anchor version semantics;
  the open vector is only a lower bound. Review integration, Machine/Plugin
  activation and independent post-effect recovery are not implied by this API.
- [Native edited-buffer coordinates](plugin-native-buffer-coordinates.md) advance
  the private Zed candidate to `1.5.0`. A bounded passive mirror uses the exact
  pinned upstream text engine for edits, undo and insertion anchors, including
  UTF-16/UTF-8 validation over tombstones. Native language, hover, navigation and
  symbol reads reject late text changes. Reload floors do not reopen owners;
  diagnostic state remains explicitly last-observed. This supersedes the `1.4.0`
  base-only restriction above, not the closed owned-read API. Review content
  certificates, destination ownership, actual Plugin activation and supported
  device acceptance remain separate.
  The [candidate acceptance](releases/plugin-native-coordinates-2026-09-16.md)
  records 51 private tests, the complete gate and exact static-process acceptance.
  Real headless Zed does not automatically reload on disk changes: explicit
  content synchronization is still missing. The candidate is unsigned and
  uninstalled; no production component activation was issued by this task.
- [Content-bound observations](plugin-content-bound-reads.md) add a closed
  complete-text identity check through Web, Controller, Machine and private
  Zed `1.6.0`. Language/symbol/hover queries refuse mismatched content before
  native dispatch and reject in-flight edit/undo ABA. Mismatch does not reload
  or replace an owner. Browser capture/typed results and native text equality
  do not grant synchronization, owned navigation destinations, ordinary Review
  integration, native installation or device acceptance.
  The [Controller/Web release](releases/plugin-content-bound-reads-2026-09-16.md)
  passed both complete gates, fourteen actual Firefox cases, static native
  conformance and identical 81-release reads by all four actual Controller
  roles. Both components are published and active; all fourteen observed workers,
  Machine and Victoria retained their processes. Machine/Zed `1.6.0` remain
  unactivated candidates, not production end-to-end Code acceptance.
- [Core browser buffer owners](plugin-buffer-client-owner.md) implement the
  typed client prerequisite: captured authority lifetime and input, one-shot
  preparation/open, strict bounded readonly observations, cancellation-safe
  continuations, and explicit original-ID cleanup. Pending/unknown/error is not
  release; ambiguous mutations do not replay, and authority loss cannot adopt
  replacement cookies. Twenty-six Web tests, two shared-wire Rust handler tests
  and six real Firefox/React StrictMode cases pass. This library is not imported
  by ordinary Review: unresolved-resource presentation,
  positional semantics, actual Machine/Code rollout and supported-device
  acceptance remain required. That library publication changed no running component.
- [Core product-context integration](plugin-buffer-product-context.md) connects
  that registry to the production auth/dataset/session-end lifetime. Sign-out
  fences remote operations before cleanup callbacks; same-Service reconnects
  preserve owners, observed replacement cannot ABA-revive them, and permanent
  socket-root abandonment seals admission before draining local writers. Ending
  authority does not drop an already-borrowed outbox's final save or send native
  cleanup with replacement credentials. Fifteen focused tests and six additional
  real Firefox cases pass, including native-IDB final-write recovery; the actual
  product-store admission fixture also retains pending prompts without delivery.
  Review, native rollout, abandoned-browser recovery and device acceptance remain
  separate; the core facade is not a serialized grant or Plugin API.
  Its [Web-only release](releases/plugin-buffer-product-context-2026-09-16.md)
  is active with the full gate passing and all 13 observed workers retained.
- [Core Code cleanup presentation](plugin-buffer-cleanup-surface.md) projects
  original retained owners into Settings with stable typed observations,
  bounded local pagination and separately confirmed one-pass cleanup. Active
  consumers cannot be closed here; uncertain release is query-only, removed
  handles cannot target replacements and authority end redacts/fences actions.
  View unmount does not cancel admitted work or replay it on remount. This closes
  the client cleanup-visibility prerequisite, not Review integration, durable
  native recovery, Machine/Plugin activation or supported-device acceptance.
  Its [Web-only release](releases/plugin-buffer-cleanup-surface-2026-09-16.md)
  passed the complete gate, nine new unit cases and thirty real-browser cases
  across four suites. It is published and active, with all fourteen workers,
  Controller, Machine and Victoria processes retained.
- [Connected core Code acceptance](plugin-code-connected-conformance.md) now
  exercises supplied immutable Controller/Machine/Zed processes through real
  disposable login, enrollment and a temporary signed installation. Three runs
  pass all seven groups: cancellation without mutation replay, independent
  content-bound owners, explicit release after a borrowed read, HTTP uninstall
  and missing-path reads, plus connection/restart refusal without adoption.
  Original native replies are held, never fabricated. Cleanup waits and reaps
  fixture descendants behind private PID/network and read-only cgroup isolation.
  Its [source-only delivery](releases/plugin-code-connected-conformance-2026-09-16.md)
  passed the full gate after integrating main. Supplied artifacts are not actual
  host-role acceptance; forced fixture teardown is not production recovery.
  The subsequent [installation and cleanup acceptance](releases/plugin-process-cleanup-2026-09-16.md)
  replaces pre-seeding with real authenticated installation into an empty slot;
  two actual-process runs pass all eight groups, including cancellation after
  the Machine receipt and duplicate-ID refusal without another install dispatch.
  Review, content synchronization, owned navigation, native rollout and
  supported-device acceptance remain open. The
  [native synchronization investigation](plugin-native-buffer-sync.md) records
  why adapter-side preflight plus upstream reload cannot safely close that gap.
- The [private native synchronization primitive](plugin-native-buffer-sync.md)
  is now implemented in a source-pinned static Zed server, with an additive
  typed protocol and new Zed `1.7.0` source manifest. Six native GPUI/filesystem
  tests include deterministic edit/undo, read-only, peer and worktree changes
  at both asynchronous boundaries, bounded source loads and retained operation
  identity. The real private-server gate verifies actual text propagation,
  stale-observation rejection, invalid source refusal, lost-response queries,
  duplicate refusal and process-instance loss. Adapter framing/correlation tests
  also pass. The [signed 1.7.0 rollout](releases/zed-native-sync-2026-09-16.md)
  includes eight-group exact-pair acceptance, compatible actual Catalog readers,
  separately authorized Hawk Machine maintenance and completed Plugin
  installation. This is not a public synchronization grant. Adapter multi-owner
  exclusion is accepted separately below; core purpose/authority, Review
  integration and independently authorized restoration remain required.
- [Private synchronization ownership](plugin-buffer-sync-owners.md) adds the
  Zed `1.8.0` release: same-native-ID owner exclusion, conservative alias/open
  admission, retained Pending/Unknown fences and one-use Apply. Original-ID
  query/retirement cannot adopt replacements, replay an effect or dispose of
  uncertainty. The verified Machine candidate explicitly denies the private
  commands before generic runtime selection; its activation awaits separate
  maintenance. Focused cancellation/sharing/capacity tests, real native-owner
  synchronization, complete gates and both actual/candidate Machine connected
  flows pass. The [signed Plugin rollout](releases/zed-sync-owners-2026-09-16.md)
  installed `1.8.0` on Hawk while retaining both original Code processes and all
  16 observed workers. This is not core writer authority, public Review cutover
  or independent recovery; the closed purpose declaration supplies none of them.
- [Core process cleanup](plugin-process-cleanup.md) no longer resolves `kill`
  through PATH. Typed process-group syscalls refuse broad/overflowed selectors,
  retain original direct-worker ownership through delivery, and treat only
  `ESRCH` as absence; permission errors retain the fence. Missing/hostile PATH,
  descendant termination, unrelated-child isolation and permission/mapping
  tests pass. This does not add PID-reuse immunity, escaped-descendant
  containment or independently verified native recovery. Controller activation
  and the initial separate Machine candidate are recorded in the same acceptance
  note; the subsequent [Hawk maintenance](releases/zed-native-sync-2026-09-16.md)
  activates this repair in Machine too, without an ambient helper workaround.
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
| P4 / capability acceptance | The core [connected installation writer](releases/plugin-install-writers-2026-09-14.md) is active and its Victoria installation/reinstall/fault matrix is accepted; the supplied Code HTTP installation/read/uninstall chain is now accepted separately. Extend this to Agent authentication projection and actual native-generation replacement | Each supported Plugin lifecycle, cancellation/crash at its additional capability boundaries, same-ID deduplication and changed-input refusal; no native restoration inferred from telemetry or forced fixture teardown |
| P4 / Code consumer | Connect ordinary Review to retained core buffer owners, explicit disk/native synchronization with dirty/shared-buffer authority, and owned navigation destinations | Actual consumer cancellation, stale text/positions, independent readers and mismatch refusal; no legacy fallback after an owned attempt or reload disguised as a read |
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

- **Historical persistence loss:** the two append rejections after the
  [1.8.0 installation window](releases/zed-sync-owners-2026-09-16.md#later-persistence-degradation--unresolved)
  led to a separately [repaired and activated Controller admission queue](releases/persistence-admission-2026-09-16.md).
  Local/public health is now 200, with zero drops/failed batches in the new
  epoch and all 16 worker/four Code process identities retained. The original
  rejected contents and recovery remain unknown; no replay or DB edit was used
  to claim restoration. The independent failed Web transaction is still awaiting
  its own recovery; no Machine maintenance was performed.
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
- **Remote core Code adapter:** the shared UTF-8 paging and owned Code changes
  are now [activated on Hawk](releases/zed-native-sync-2026-09-16.md) through the
  separately authorized Machine release, with exact connected acceptance and
  installed Zed 1.7.0 evidence. The [subsequent 1.8.0 Plugin-only upgrade](releases/zed-sync-owners-2026-09-16.md)
  retained that Machine and existing Code connections; its new explicit generic
  synchronization guard is still a verified Machine candidate. Ordinary Review,
  independent recovery, other Machines and supported-device acceptance remain
  separate; a healthy Machine
  and installed Plugin do not establish those consumer exits.

These checks require real account/device participation and, for the Machine or
host-policy change, the separate maintenance boundary. Synthetic fixture keys,
isolated databases and an agent's filesystem access cannot substitute for them.
The physical iPhone pasted-image caret issue in `web/src/mdlive/PITFALLS.md` #69
also remains unsolved; it is not a Plugin-refactor completion claim.

## Completion rule

Do not report the whole refactor complete until the code exits and the production
and supported-client exits above have recorded evidence. Preserve the distinction
between implemented, verified, published, activated and actually accepted.
