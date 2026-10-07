# Plugin refactor exit status — October 5

This is a dated reading of [the completion ledger](plugin-refactor-completion.md)
against the code and Hawk's production receipts. It adds no acceptance of its
own; where the ledger and an older release disagree, this page names which was
re-checked. The refactor is **not complete**: the code exits and the real
account, device, native-generation and policy exits below remain open.

Production at this reading: Machine writer release `9c79b9c3` (transaction
`1791179356912329897-9c79b9c379ec`, worker generation `worker-748825b4…`,
retained worker source `b97c2724…`); Controller and Web were moved by independent
tasks. **Update 2026-10-06 (later):** the incarnation writer is also active on Hawk
([release](releases/incarnation-writer-2026-10-06.md)); steps 1-3 of the durable
incarnation design are done and step 4, carriage to the Controller with stale
observation refusal, is implemented and tested but held on a local branch because
its wire change would block host-only Machine releases until the worker pin
advances ([record](plugin-session-incarnation-carriage.md)). Nothing consumes the
lineage in production yet. **Update 2026-10-07:** the carriage and the pin advance to
it are on `main` (`f469cf04`) and active on Hawk's Machine and Controller
([release](releases/worker-pin-incarnation-carriage-2026-10-07.md)); no live worker
was drained, and connected Code still fails at a stage that predates this work.

**Update 2026-10-06:** another task completed the worker-pool maintenance (Machine
`48d3054d`, `worker-135348a7…`, pin `8909c1c8`) and the items below are now active;
the conformance on built artifacts passed (37 groups, see the
[reader contract](plugin-session-incarnation-reader.md)). The incarnation floor is
still unanchored. The paragraph below is the 2026-10-05 reading.

Source on main but **not active** because host-only Machine releases are blocked
(main carries the independent wire change `22de6bbf`, so the retained worker pin
must advance first, which drains every live Hawk worker): the
[incarnation reader](plugin-session-incarnation-reader.md), bounded in-process
cleanup retries, and the shared `namespace.rs` refactor. The refactor touches the
active deletion journal, so its native old/new writer conformance must be re-run on
a built artifact before it ships. The
[worker-pool candidate](releases/machine-pool-candidate-2026-10-05.md)
(`worker-5e009066…`) has Codex execution, logs, session and Codex/Claude
(Hawk-installed) coexistence accepted; Claude native execution (needs the
3.4.7 runtime built) and connected Code (needs the exact Zed pair) are open. The
Columbus owner knows `sessionIncarnations` on Hawk only; Falcon is untouched and
need not change until a release declaring the dataset is meant for it.

State words follow the ledger: **implemented**, **verified** (automated
gates), **published**, **activated** and **accepted** (the stated real-world
exit, not a proxy).

## Exits

| Exit | Implemented / verified / activated | Still missing, and what would close it |
| --- | --- | --- |
| A. P0 typed resolution, continuous identity | Finite release/Catalog, Service/Machine Site, telemetry and Code-read observations; Machine root/workspace identity (protocol 24/25) and Controller-local root identity; process-local lifecycle fence; terminal deletion journal and Hawk writer; **cleanup continuation (this release)** | General graph/Site/state-lease contracts; continuous Machine-owned Session/worktree/security-domain identity from launch through reset and termination; **durable Session incarnation**; state leases/policy ([design](plugin-state-lease-design.md): no consumer exists, so deliberately not built); binding resolved results to finite domain executors ([rule and inventory](plugin-domain-executor-binding.md): done for telemetry, the only domain with such an executor; nothing generic by design); portable deletion-writer admission; more refusal/cancel/change/independent-observation vectors |
| B. Session cleanup | Preparation containment, original-root retention, bounded markers, target handles, `openat2` scans, descendant handles, leaf identity, deferred marker finalization, retry identity and marker progress — all activated; **durable continuation activated, startup/admission accepted, not exercised live** | Two non-transactional marker unlinks; non-atomic final identity comparison and name unlink; unfrozen same-inode/concurrent content; completed paths mean "original scan", not "empty now"; a hung system call is not interrupted (per-pass wall-clock budget exists in source, see the cleanup-targets contract); non-Linux fallback ordering; cross-Session retained-resource budget; crash/power-loss proof; a live deletion→restart→resume observation |
| C. P3 state compatibility | Finite security, telemetry and browser dataset namespaces; [design](plugin-state-dataset-design.md) written, unimplemented | General state-dataset identity; actual old/new reader and writer coexistence; exclusive fenced ownership; principal change, crash/reopen, version change; independent workspace/generation coexistence; active, next-recovery and cold immutable readers. New migrations only; the deployed SQLx baselines stay byte-exact |
| D. P4 Agent/Plugin capability | Connected installation writer and Code HTTP chain accepted | Agent authentication projection lifecycle; actual native-generation replacement; each capability's install/upgrade/uninstall, cancel, crash and partial-effect boundary; same-ID dedup and changed-input refusal |
| E. P4 Code consumer / native budgets | Finite sync/reload/remote-edit budgets, acquisition and snapshot lifetimes, single-use open and close confirmation | Global retained-history, snapshot and background-effect budgets for every writer; deployed Machine with the exact native/client pair accepted independently; owned Review consumption, cancel, stale text/positions; replacement of retained Sessions across generations. Production private navigation policy stays closed; it is not opened to manufacture a pass |
| F. P4 recovery and evidence archival | Observation and Unknown retention; local disposal only | Independently authorized post-effect Plugin restoration; exact native-worker recovery and owner migration; installation CAS with fresh recovery purpose and budget; archival that keeps unresolved references. No restored-turn claim, no credential rollback, no user-data deletion |
| G. P4 diagnostics / UI | Unified install/uninstall/resolution Web consumer is live for the current finite domains | Extend as new domains and recovery exist; reload shows durable status without replay; no cross-domain atomic snapshot claim |
| H. Production, account and client exits | See below | Real account/device/Operator/policy evidence; none can be supplied by filesystem access or synthetic fixtures |
| I. iPhone pasted-image caret | Unsolved | Physical-device verification after reading `web/src/mdlive/PITFALLS.md` #69; no claim of a fix |

## H, item by item

- **Historical persistence loss:** unchanged. The admission queue repair and a
  healthy new epoch do not recover the rejected contents.
- **Failed Web transaction `1789566575530411524-8edf513e0307`:** re-checked here.
  Its history holds `…-recovered-rolled-back.json` (`recovered-rolled-back`,
  `rolled-back`, `published=false`, `2026-09-17T01:32:34Z`). The earlier ledger
  statement that it was still awaiting recovery was stale and is corrected. This
  is a rollback to the previous release; it delivered nothing and restored no
  browser data. Older `failed` Controller/Machine transactions without a
  recovered receipt remain historical and are not individually reconciled.
- **Supported browser/device reload, storage, origin and gesture:** open.
- **Retained worker generation:** worker PIDs persist across Controller, Web and
  resident Machine activations, which proves neither generation replacement nor
  native resume. A swap is a separate maintenance acceptance. Old browser records
  stay unowned.
- **Core security handoff** (real Password, device authorization, Passkey, native
  origin/gesture, then owner-committed host policy and stopped-Controller
  adoption): open. Generated local-auth pins and authority markers are not
  removed, and old public SDK/native entries are not retired.
- **Managed Victoria cutover** (Operator confirmation, two writer policies,
  standing export policy, private destination authentication/TLS, real
  ingestion/query and controlled failure): open. The first managed intent fences
  legacy export permanently, including a rejected or aborted one.

## Design finding that orders A and C

The cleanup continuation could ship without a reader floor or owner change
because it is monotone and liveness-only: an older resident ignores it, and a
newer one still needs the committed permanent deletion, the exact original root
and a fresh scan before any effect. Losing it costs only the old behaviour.

A durable **Session incarnation** is the opposite kind of dataset. Its job is to
refuse stale observations, so a Machine that does not know it (a rollback, a
reader-only fallback, a portable launcher) would silently stop refusing, and a
stale value could then be revived. It therefore needs what the deletion journal
has: a closed schema, a root-owned reader floor per dataset, writer admission,
exclusive fenced ownership, an unknown-outcome rule and rollback refusal, plus a
protocol field that carries the Machine-minted value to the Controller. The
installed owner (`machines/internal/cowboyrelease`) currently binds one floor to
one dataset path, so the dependency order is:

The full contract is the [incarnation design](plugin-session-incarnation-design.md).

1. Generalize the owner's floor and admission to a dataset key, reader-only first,
   without changing the deletion dataset's bytes or behaviour (Columbus work,
   separate maintenance acceptance). **Done on Hawk** (Columbus `5af9f70b`,
   host transaction `1791181417007228582-5af9f70b5281`): the owner now knows an
   optional `sessionIncarnations` declaration, a per-dataset floor and
   committed-state admission; undeclared artifacts stay valid until a floor or
   state exists. Not exercised by a Cowboy release yet and **not activated on
   Falcon**, which keeps its older owner until its own host activation.
2. Ship a Machine reader for the incarnation namespace with the writer disabled.
   **Implemented, verified and published** (`4d792318`, see the
   [reader contract](plugin-session-incarnation-reader.md)); **not activated**:
   main now carries an independent wire change that host-only Machine releases
   refuse, so activation waits for that task's separately accepted full Machine
   maintenance.
3. Admit a writer behind the floor; mint and persist an incarnation per Session
   slot before launch, rotate it on reset, supersede it on deletion.
4. Carry it through the Machine protocol and make the Controller's existing
   process-local `SessionCodeScope` incarnation an observation of it, with
   mismatch refusal and independent reader vectors.

Steps 1–4 each need their own old/new reader and writer acceptance; none of them
should be bundled into an ordinary Controller, Web or Resident fix. Step 4 is
where state leases attach, so the P3 general dataset identity should reuse step 1
rather than add a second floor mechanism.
