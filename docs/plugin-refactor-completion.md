# Plugin refactor completion ledger

Reviewed against the 2026-09-09 [target architecture](plugin-spatiotemporal-design.md)
and current code on 2026-09-18. This is the current exit checklist, not a list of
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
- [Machine synchronization continuation](plugin-machine-buffer-sync.md) adds
  protocol-20 core routing and a non-serializable original-connection invocation,
  separate from generic Code forwarding. The Zed `1.9.0` source candidate adds
  a distinct ownership-support probe; native-only support cannot satisfy it.
  One-use Apply, unknown exclusion, original-process retention and bounded
  observation/retirement are implemented. Source tests cover cancellation,
  connection/queued-authority loss, malformed replies, capacity and runtime death.
  Its [candidate acceptance](releases/machine-buffer-sync-candidate-2026-09-17.md)
  passes the complete gate, exact static native/core synchronization across
  uninstall, eight connected Code regressions and six Nix checks. Publication,
  installation and Machine activation remain separate. That earlier candidate
  did not supply Service confirmation; the following slice adds it. Ordinary
  Review integration and independent recovery are still not enabled.
- [Service synchronization continuations](plugin-service-buffer-sync.md) add
  explicit original-owner preparation and fresh product confirmation on protocol
  20, with typed Site-bound transport, one-use Apply, independently owned jobs,
  conservative uncertainty and path-free original-ID cleanup. Authority and
  Session/connection checks cannot be replaced by serialized IDs. Ordinary
  buffer admission now clears expired inert Machine preparations too, without
  clearing possible effects. Response-authorization job exclusion also prevents
  an older Drop from reopening a successor's admission. Its
  [Controller-only rollout](releases/service-buffer-sync-2026-09-17.md) passes
  the complete gate, two actual 11-flow immutable process runs and all eight Nix
  checks; 15 original workers and four native Code processes survive activation.
  The resident Machine and installed Zed remain unchanged, so this is not
  production end-to-end synchronization. The browser follow-up below now adds
  ownership/presentation; ordinary Review, signed Plugin rollout, separate Machine maintenance and
  independent restoration remain required.
- [Browser synchronization ownership](plugin-browser-buffer-sync.md) connects
  the Service continuation to the actual core product identity lifetime and
  bounded Settings presentation. Complete-text captures and one-use opaque
  confirmation tokens bind the original buffer, content and observation;
  cancellation, remount and stale confirmation cannot replay Apply. Unknown
  effects fence ordinary buffer cleanup, terminal evidence is immutable, and
  retirement is explicit rather than an undo. Closed Rust/Web wire fixtures,
  focused unit/compile tests and an eight-case isolated React/browser gate
  cover these boundaries. Its [Web-only release](releases/browser-buffer-sync-2026-09-17.md)
  passed the full gate and 38 browser cases, is published and active, and
  retained all 16 observed workers plus four native Code processes. This does
  not itself switch ordinary Review or activate a Machine/Code Plugin; production end-to-end synchronization, navigation,
  independent recovery and supported-device acceptance remain separate.
- The [ordinary Review source consumer](plugin-review-owned-consumer.md) now
  selects core-owned language, hover and Outline from a protocol-20 Service
  observation. Exact displayed LF content, serialized/cancellation-safe reads,
  original-ID Open checks, explicit refresh preparation and stale Apply fencing
  share one owner without legacy fallback. Its
  [Controller/Web delivery](releases/review-owned-consumer-2026-09-17.md) passes
  the full gate, 44 browser cases, two actual eleven-group process runs and Nix
  checks; all 16 original workers and four Code processes survive activation.
  That delivery retained the then-protocol-19 Machine and Zed 1.8; it did not
  accept native/device cutover or owned navigation/diff integration.
- [Owned working-diff reads](plugin-review-owned-diff.md) now validate complete
  patches against bounded complete current-file text and map only exact new-side
  UTF-16 positions. Projection replacement ends old observers even for equal
  source text; deleted/header taps cannot borrow nearby symbols. Staged/history
  views remain render-only, and no owned diff uses legacy resource operations or
  prepares synchronization. Its [Web-only delivery](releases/review-owned-diff-2026-09-17.md)
  passes the full gate, 55 browser cases and two eleven-group native runs;
  all 16 workers and two observed Code processes survive activation. The already
  deployed protocol-20 Machine is unchanged, and production Zed remains 1.8.
  Native, cross-file navigation and device acceptance remain separate.
- [Owned native navigation](plugin-owned-navigation.md) adds a Zed `1.10.0`
  source candidate with preallocated group identities, one-use native queries,
  exact target ID/content/epoch retention, path-free local release and conservative
  Unknown admission fences. Same-native-ID aliases now prevent premature peer
  closure. Generic Machine forwarding refuses the three private commands before
  selecting a runtime. This is not a Service/Web navigation cutover, verified
  native cleanup, independent recovery or production native-generation acceptance;
  original-runtime routing and actual nonempty-LSP consumer acceptance remain open.
  Its [candidate acceptance](releases/owned-navigation-candidate-2026-09-17.md)
  passes the full gate, static builds, signed temporary lifecycle and all eleven
  connected Code regressions; the actual plaintext navigation gate does not
  establish nonempty-LSP acquisition, native allocation bounds or recovery.
- The Zed `1.11.0` [private handoff extension](plugin-owned-navigation.md#exact-destination-handoff)
  adds a bounded ordinary-owner reservation from an exact retained navigation
  target. Atomic handoff performs no native/path I/O and survives parent release;
  cancelled waits, lost replies, equal-content ABA and capacity retain their
  original identities. A real nonempty stdio-LSP fixture exposed and now covers
  missing native target registration, and the static-pair gate exposed locations
  arriving before their asynchronous target shares. Execute waits for the exact
  original shares and registers newly retained targets once, with failure
  retaining Unknown and original pins. Its
  [candidate acceptance](releases/owned-navigation-handoff-2026-09-17.md) passes
  four nonempty-LSP static-pair runs, temporary signed lifecycle, all eleven
  connected regressions and complete source gates after integrating main. This
  is not core acquisition authority, Machine/Service/Web navigation routing, a
  native allocation budget, independent recovery or a production rollout.
- [Machine navigation continuation](plugin-machine-buffer-navigation.md) adds
  protocol-21 Site/connection-bound acquisition and exact-runtime route ownership.
  Its separate `navigation:` identities retain one-use execution, query-only
  uncertainty and release, bounded inert expiry and historical target evidence.
  Exact destination reservations join the ordinary buffer lifecycle/capacity,
  with saved lookup after observer loss and no path/installation reselection.
  Zed `1.12.0` adds the distinct actual-pair support probe; native dependencies
  are unchanged. Its [candidate acceptance](releases/machine-navigation-candidate-2026-09-17.md)
  records 22 new core tests, complete gates, static native and signed-lifecycle
  nonempty-LSP acceptance, and eleven connected buffer regressions. Independent
  review also closed expired inert synchronization fences blocking direct
  navigation; uncertain effects remain retained. That milestone leaves
  Service/principal/Session authority to the subsequent candidate below.
  Public cutover, native allocation budgets, independent recovery and production
  rollout remain separate; neither completes the Code-consumer or general DAG exits.
- [Service navigation continuation](plugin-service-buffer-navigation.md) adds
  default-closed, product Operator/Session-bound acquisition on protocol 21.
  Original connection and one-use outcome records survive HTTP observer loss;
  unknown acquisition/release is query-only. Destination preparation enters the
  ordinary buffer owner without implicit Open or path fallback, retaining its
  original capacity/TTL through uncertainty. The connected gate advances to v4
  and 17 required checks, including nonempty Unicode navigation, lost replies,
  handoff after uninstall and independent reads after parent/path removal. Its
  [candidate acceptance](releases/service-navigation-candidate-2026-09-17.md)
  records the complete final source gate, unchanged exact native pair, supplied
  Controller/Machine and two successful 17-check connected runs.
  Full destination views, native budgets, Web integration, rollout and recovery
  remain separate.
- [Complete original native text](plugin-native-text-reads.md) adds the finite
  Zed `1.13.0` reader through Machine, Service and the typed browser owner.
  Bounded UTF-8 pages retain the original owner/revision, refuse content or
  snapshot changes and never reopen a path. The browser returns only a complete
  SHA-256-verified capture, drains cancellation and stops further page admission.
  Its [candidate acceptance](releases/native-text-reads-candidate-2026-09-18.md)
  passes the complete source gate, static native and temporary signed-lifecycle
  gates, 49 isolated browser cases and two v5 connected runs with all 18 checks.
  These include two-page Unicode reads after uninstall, parent release and path
  removal. Navigation acquisition remains default closed; this reader does not
  supply the intended Review destination owner/view, native allocation bounds,
  production rollout, supported-device acceptance or independent recovery.
- [Browser navigation continuation](plugin-browser-buffer-navigation.md) adds
  core original-source preparation, one-use Execute, query-only uncertainty and
  separately observed Release. Source cleanup cannot bypass a retained group;
  bounded local capacity includes pending preparation, and only inert expiry or
  actual group release frees a slot. Closed Rust/Web wire evidence, disjoint
  types and a passive recovery projection reject imported destination IDs and
  redact ended identity labels. Its
  [candidate record](releases/browser-navigation-candidate-2026-09-18.md) covers
  complete source/Nix gates, 56 browser cases (18 in the owner suite) and all
  18 connected v5 checks against the unchanged exact native pair. Destination handoff,
  intended Review views, native budgets, production activation and independent
  recovery remain separate; acquisition is still default closed.
- [Browser destination handoff](plugin-browser-buffer-destinations.md) adds
  original opaque target tokens, ordinary-owner capacity reserved before the
  single POST, and closed Service receipt adoption into that same local slot.
  Query can reconcile lost replies without another preparation; view close
  retains unresolved ownership. Open and child release remain explicit and
  independent of group/source cleanup; historical Prepared receipts cannot
  reset or revive a child. This closes browser preparation/adoption, not the
  intended Review destination view, native allocation bounds, device acceptance
  or production acquisition policy. Full source/Nix gates, 62 browser cases,
  the exact static pair and all 18 connected v5 checks pass. Input review also
  repairs the upstream private-runtime version mismatch in a Zed `1.13.2`
  candidate, without publishing or installing it. Verification is recorded in the
  [candidate acceptance](releases/browser-destination-candidate-2026-09-18.md).
- [Single-use native Open](plugin-native-open-once.md) advances the Zed candidate
  to `1.14.1` without changing private server `1.1.0` or third-party pins. Missing
  initial sharing no longer triggers implicit Close/reopen. A one-use original-
  owner commit ends the acquisition fence; cancellation/error retains it even
  before the native ID is known. New opens, navigation, synchronization and
  destination handoff cannot bypass it, while known safe reads/releases remain
  available. Its [candidate acceptance](releases/native-open-once-candidate-2026-09-18.md)
  passes 115 adapter tests, actual normal-timeout/original-peer and immutable
  socket failure checks, temporary signed lifecycle, all 18 connected v5 and
  24 browser checks, plus the complete source/Nix gates. This is not native close
  acknowledgement, independent recovery or a production native rollout.
- [Original-peer native close](plugin-native-close-confirmation.md) advances the
  Zed candidate to `1.15.0` / private server `1.2.0`. Ordinary last-owner and
  navigation-group release now require an exact original-instance native batch
  confirmation before local pin removal. Lost native ACKs retain uncertainty,
  original pins and exclusion with no replay; unrelated confirmed releases
  cannot clear that fence. This verifies native peer-map removal, not all buffer
  deallocation, background-effect drain, recovery or a production rollout.
  Its [candidate acceptance](releases/native-close-confirmation-candidate-2026-09-18.md)
  passes 124 adapter tests, 19 native tests, the exact static pair, temporary
  signed lifecycle, all 18 connected v5 and 24 browser checks, plus the complete
  source/Nix gates. Source-test loss of a real native Closed reply remains
  Unknown; a completed upper-layer release still settles by original Query.
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

The [ordinary Review source consumer](plugin-review-owned-consumer.md) now selects
owned language/Outline/hover and explicit refresh preparation from a protocol-20
Service observation. It retains content/owner lifetimes without legacy fallback;
protocol-19 Machines remain on their pre-cutover route. This is not owned
navigation, native activation or supported-device acceptance. The later
[working-diff integration](plugin-review-owned-diff.md) supplies explicit
complete-current-file projection without adopting staged/history coordinates.
The [source Controller/Web release](releases/review-owned-consumer-2026-09-17.md)
and later [working-diff Web release](releases/review-owned-diff-2026-09-17.md)
record exact accepted artifacts and bounded production continuity observations.

| Exit | Remaining implementation | Required evidence |
| --- | --- | --- |
| P0 / typed resolution | Extend verified release observations, finite Service/Machine Site checks, telemetry resolution and code-read observations to applicable graph contracts, continuous Machine-owned Workspace/Session/security-domain identity, state leases and policy; link exact resolved results to finite domain executors | General graph/site/state-lease vectors beyond accepted-Catalog, finite Site, code-reader and telemetry installation fences and shared structural link vectors; no serialized authorization |
| P3 / state compatibility | General state-dataset identity and reader/writer coexistence beyond the finite security, telemetry and now-deployed browser namespaces | Actual old/new readers and writers, exclusive fenced ownership, principal changes, crash/reopen, version-change and independent workspace/generation coexistence |
| P4 / capability acceptance | The core [connected installation writer](releases/plugin-install-writers-2026-09-14.md) is active and its Victoria installation/reinstall/fault matrix is accepted; the supplied Code HTTP installation/read/uninstall chain is now accepted separately. Extend this to Agent authentication projection and actual native-generation replacement | Each supported Plugin lifecycle, cancellation/crash at its additional capability boundaries, same-ID deduplication and changed-input refusal; no native restoration inferred from telemetry or forced fixture teardown |
| P4 / Code consumer | The [Review destination reader](plugin-review-owned-destinations.md) connects explicit navigation, original-target handoff, independently owned native-text display and passive Settings recovery. The [native input candidate](plugin-native-input-bounds.md) bounds single inputs; the [whole-query candidate](plugin-native-navigation-budgets.md) adds aggregate pre-acquisition location/target budgets, original-worktree-only opens and typed refusal. [Single-use Open](plugin-native-open-once.md) removes implicit replay and fences unobserved acquisition; [native close confirmation](plugin-native-close-confirmation.md) verifies original-peer removal. Global retained-history/background-effect limits and independent acceptance of the deployed Machine/exact native pair remain open | Actual deployed consumer cancellation, stale text/positions, independent readers and mismatch refusal; no legacy fallback after an owned attempt or reload disguised as a read; aggregate native lifetime/resource limits and supported-client native acceptance |
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
  synchronization guard was then a verified Machine candidate. The later
  [working-diff release observation](releases/review-owned-diff-2026-09-17.md)
  records an already-active protocol-20 Machine and retained Zed 1.8, not
  installation of the exact 1.9 native pair used by the owned-buffer gates.
  Actual owned Review acceptance,
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
