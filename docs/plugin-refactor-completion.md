# Plugin refactor completion ledger

Reviewed against the 2026-09-09 [target architecture](plugin-spatiotemporal-design.md)
and current code on 2026-09-14. This is the current exit checklist, not a list of
all historical slices. Passing a test, publishing a reader, enabling a policy
and accepting a production effect are different milestones.

## Implemented and accepted in automated gates

- One immutable signed Plugin lifecycle and dependency-closure release matrix;
  six independent Agent sources plus Zed, with no installable internal runtimes.
- Core-owned communication, installation, security/native/Web host boundaries;
  isolated Provider accounts, exact session/workspace generations and sidecars.
- Generated closed composition wire types and bounded, read-only structural
  checkers in Rust and TypeScript; combined ownership/capability cycle checks
  and an 86-case real-CLI differential gate compare complete link reports. This
  is not verified live resolution or a grant.
- Typed Provider authoring, owned UI effects, state-store/resource scopes,
  subscription/task drain and explicit IDB connection/transaction ownership.
- [Atomic IDB outbox deltas](atomic-idb-outboxes.md) preserve updated peers'
  pending mutations and confirmations in one transaction, with explicit load
  handoff and strict durable-send failure. The v1 data format is unchanged;
  pre-upgrade blind writers and general dataset authority remain unfenced.
  The [Web release](releases/atomic-idb-outboxes-2026-09-14.md) passed both
  real-browser suites and activation checks without restarting live workers.
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
- Finite Victoria binding/revoke/restore, independently authorized recovery and
  settlement, bounded managed OTLP export, standing policy and private admission.
  Actual immutable role gates cover 96 reader, 294 writer, 78 startup and 45
  connected-flow cases; [real Victoria acceptance](releases/telemetry-victoria-conformance-2026-09-14.md)
  adds nine database-backed role pairs and reopen/no-replay checks.

## Code work still required

| Exit | Remaining implementation | Required evidence |
| --- | --- | --- |
| P0 / typed resolution | Resolve applicable composition contracts against verified releases and actual enrolled sites, state leases and policy; link that exact resolved result to a finite domain executor | Verified-release/site/state-lease vectors and live-precondition changes, beyond the accepted shared structural link vectors; no serialized authorization |
| P3 / state compatibility | General state-dataset identity and reader/writer coexistence beyond existing core-security and telemetry namespaces; version-fenced IDB ownership across old/new clients, beyond accepted updated-peer atomic deltas | Old/new readers and writers, exclusive fenced ownership, principal changes, crash/reopen, version-change and independent workspace/generation coexistence |
| P4 / capability acceptance | The core [connected installation writer](releases/plugin-install-writers-2026-09-14.md) is active and its Victoria installation/reinstall/fault matrix is accepted; extend actual immutable process acceptance to Agent authentication projection and code/native integration | Each supported Plugin lifecycle, cancellation/crash at its additional capability boundaries, same-ID deduplication and changed-input refusal; no native restoration inferred from a telemetry fixture |
| P4 / recovery | Independently authorized post-effect Plugin restoration and verified exact native-worker recovery; bounded evidence archival that retains unresolved references | Installation CAS, fresh recovery purpose/budget, partial outcomes, no restored-turn claim, no credential rollback or userdata deletion |
| P4 / diagnostics | One typed projection of these real operation instances and their independent recovery actions | Reloaded UI reads durable status without replay or fabricated completion; recoveries remain explicitly separate |

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

These checks require real account/device participation and, for the Machine or
host-policy change, the separate maintenance boundary. Synthetic fixture keys,
isolated databases and an agent's filesystem access cannot substitute for them.
The physical iPhone pasted-image caret issue in `web/src/mdlive/PITFALLS.md` #69
also remains unsolved; it is not a Plugin-refactor completion claim.

## Completion rule

Do not report the whole refactor complete until the code exits and the production
and supported-client exits above have recorded evidence. Preserve the distinction
between implemented, verified, published, activated and actually accepted.
