# Dataset-bound activation and compatible recovery floor

The finite [product dataset](../product-sync-datasets.md) deployment sequence is
active on Hawk. This supersedes the deferred Web/Machine/cold-floor status in the
[Controller bridge release](product-datasets-and-lifecycle-2026-09-15.md), not the
remaining general Plugin DAG, recovery or supported-client acceptance exits.

Cowboy `869c269f` removes the old-browser compatibility switch. Cookie and native
shell clients without a dataset now receive 426; supplied mismatches remain 409,
and authenticated CLI clients keep their separate class boundary. Communication,
installation, authentication and native hosts remain core-owned. No Plugin source,
version or Catalog pin changed, and no Plugin was installed by this maintenance.

## Published source and actual outputs

Cowboy main advanced concurrently with a separately authored frontend fix. The
first 869c269f Controller activation was correctly refused as stale, before a
root transaction or production change. The task fast-forwarded to
`10898c8214357142bd7410537eb99e0f8a3bf3c8`, repeated the complete gate and browser
checks, rebuilt, and accepted the integrated Controller. Both candidate releases
resolve to the same actual ELF:
`/nix/store/fmr30vwybbml00qbzf7nv5mvi350gbzh-cowboy-0.1.0/bin/cowboy`.

| Role | Source | Immutable release |
| --- | --- | --- |
| Controller active / next recovery | `10898c82` | `/nix/store/8zlgchrpyxd01ba037whwsk4zp3l0bwx-cowboy-controller-release` |
| Machine active / next recovery | `6a1eff6b` | `/nix/store/5wh7wikgqbl8r4ya927xh04dizqmjziq-cowboy-machine-release` |
| Web current, subsequent concurrent release | `10898c82` | `/nix/store/7dczgy5nm0j3iz5szpi2in5l1i7djvb8-cowboy-web-release` |
| Controller cold | `869c269f` | `/nix/store/cc09k6l788mhchy321ckgg0yryb1hg12-cowboy-controller-release` |
| Machine cold | `869c269f` | `/nix/store/j7lix2f4wbp2dvbxs7hprmp5kzcr413n-cowboy-machine-bootstrap-release` |
| Web cold | `869c269f` | `/nix/store/hmf3v0p3rspw7gc95x9h70nq6synp8qa-cowboy-web-release` |

The final host source is published Columbus
`80e6788785d0834154c40ae498b11128017a95ee`, with active system
`/nix/store/6d1n1i0c44zjzvi2dxr1r8dxlsrr404k-nixos-system-hawk-26.05.20260731.5b4f72e`.
The cold pin deliberately retains the accepted bound reader; it is not a claim
that the cold Web contains the later unrelated composer fix. The actual Nix
followed-input bootstrap outputs, not standalone Cowboy approximations, were
bound to every cold-role fixture and checked against the activated closure.

## Compatible first Web recovery

The Columbus transaction now accepts an independently verified same-lane recovery
release for **Web as well as Controller**. Before the first IDB-v2 Web activation,
its ordinary predecessor was an incompatible blind writer. An accepted Web
recovery must retarget both the profile and asset link; even a profile restoration
failure must not serve the old assets. Missing, replaced, wrong-source or
cross-lane recovery fails closed without silently choosing the predecessor.

This extends the existing journaled component lifecycle, not a second installer
or runtime. Tests cover ancestry and lane constraints, interrupted journal
reopen, receipt propagation and profile restoration failure. Machine recovery
selection and failed-transaction repair remain separate; the latter is still
Controller-only. Retain a compatible activator in host recovery too: an old
helper cannot safely consume the new explicit-Web-recovery receipt.

The task's first Web release was
`/nix/store/l4cyr8sycg7a6pap2bnfv1m2nydklqrq-cowboy-web-release`, source `6a1eff6b`,
SW v1688. It explicitly selected accepted recovery
`/nix/store/iw05ca344w6dfffyl5sa3z7w9gbbhqm5-cowboy-web-release`, whose complete
assets and source manifest matched. A subsequent concurrent task published and
activated SW v1689 at `10898c82`; this maintenance preserved it and verified its
actual local/public files. The current `/version` is
`1d5c2621b970fc36281623af54c4f8f5`. Stale PWAs/native shells still need a hard reload.

## Transactions and observed process scope

| Transaction | Purpose |
| --- | --- |
| `1789427477400991914-b6fd1e7077f8` | Initial compatible bridge/Web cold floor and Web-recovery activator |
| `1789427669094224158-6a1eff6bfbdd` | Independently authorized resident Machine maintenance |
| `1789427773928567388-6a1eff6bfbdd` | Dataset-aware Web and unified lifecycle-history consumer |
| `1789429447140143805-10898c821435` | Main-integrated binding-required Controller |
| `1789429599671515654-80e6788785d0` | Final binding-required cold recovery floor |

All five transactions succeeded. Component receipts are committed. The initial
host receipt truthfully records `published=false` at dispatch; its source was
subsequently pushed, and the final host receipt records `published=true`.
Historical receipts were not rewritten. Future Controller recovery is its now
active release, not the latest receipt's historical `previousRelease` bridge.

Full merged unit inspection preceded both host transactions. AccountsService and
polkit restarted because generated system paths changed; D-Bus reloaded. Their
policy content did not change. Cowboy, resident Machine, worker and Victoria
process identities stayed unchanged across each host transaction. The bound
Controller transaction changed only its own daemon. The Web transaction changed
none of those process identities; unrelated `greetd` startup in its broader
observation window is recorded, not attributed to Web activation. No new failed
units appeared. Clearing pre-existing failure markers in the initial host switch
is not a claim that unrelated backup/portal problems were repaired.

The Machine reports online/connected with desired ACP generation
`worker-48ad34f5c4615668b75f`. Its real startup policy preflight was unconfigured.
All 14 worker PID/start/generation tuples survived the last complete pre-dispatch
capture at 07:14:17.305 through the immediate post-capture at 07:15:02.745 +08:00.
One worker had independently changed before that admission capture; the broader
07:13:46 preflight is not described as unchanged. Retained old worker generations
were not forcibly rebound, and not every worker is therefore on the patched TLS
dependency. No native restore or subsequent-turn acceptance follows from these
process observations.

## Native observation correction

The strict settled native-ID audit failed because one Codex session acquired a
new ID. Further activation paused while the task investigated. Bounded logs show
a revive at 07:15:08.833, followed by an explicit Cowboy `context_cleared` marker
at 07:15:15.081 and a new native creation at 07:15:16.484. Production emits that
marker only from the client reset handler. This refutes the initial diagnosis
that the observation demonstrated automatic-rollout native-ID loss.

The failed strict audit remains failed, with no success receipt. The independent
reset is excluded from native-continuity acceptance; the other 13 workers retain
their process identities, 12 have unchanged current-PID native-ID hashes, and one
lacks matching lifecycle metadata. No broker/worker resume code was modified,
no old native ID was restored and no Provider home, credential or transcript was
read. The diagnostic read only bounded Cowboy event-marker metadata and kernel
lifecycle fields; the task did not clear any production history.

## Verification and retained evidence

The canonical repository release gates and complete built-system inspection
determined the staged deployment order and compatible recovery selection.
`just check-compact` passed both before and after main integration: 1182 Rust
tests, 285 Machine tests, 1481 Web tests, 17 PostgreSQL cases, 86 differential
composition vectors, all maintained feature/native/package checks, format,
Clippy, dependency policy and release builds. Intentionally ignored tests remain
29 Rust and two Machine cases. Both clean-source browser runs passed eight IDB,
sixteen outbox and six real React lifecycle-history cases with fresh isolated
Firefox profiles; no production login or physical device was used.

Nine staged role matrices each passed 420 checks: six actual Controller
dataset/history cold opens, 168 Service installation readers, 72 Machine
installation readers, 96 telemetry readers and 78 background-startup cases.
That is 3,780 checks across repeated role matrices, not 3,780 distinct scenarios.
The final actual Controller active/next-recovery/cold modes are all **bound**.
The Columbus complete gate and clean owned host builds also passed. Local and
public health/version, five exact Web/admin/asset files and no-store/immutable
headers passed after the final switch; runtime handoff and pending commands are
zero in the final capture. Synthetic fixtures did not authorize production
Plugin installation, account login, telemetry-policy changes or external export.

Private proof root: `/tmp/cowboy-dataset-maintenance-qHgjltjv`. It retains failed
attempts and corrected scope assessments alongside the successful receipts.

| Evidence | SHA-256 |
| --- | --- |
| `cowboy-check-main.log` | `90aa882f6df8fac26a7e9d122f5344fcfe53b44cdae247767ece757617ec9ddf` |
| `browser-main.log` | `588774e17bf9a423fe86fc010c588f900a1776e6cb9187df89da418c13dea602` |
| `final-evidence.json` | `fc0b2cac2231bdffa5e6156c9a7f58ba937672e20e3fa422916c56bc17fabe7c` |
| `main-activation-audit.json` | `d1ee4ad8e4970480aa0bf3e56f0d1dda16a526b525b6c2070c1bdb971466d76d` |
| `floor-activation-audit.json` | `022895c8684abbeaa1ba2b010e15d7e23f5908f43956fb5ded93f9a4d5c3d6b8` |
| `worker-reset-assessment.json` | `fda7be304b487b8321dbcf31fdfb3064abefddb54391546bc50742f0283238df` |

The [completion ledger](../plugin-refactor-completion.md) still requires general
live graph/state leases, capability-specific and native acceptance, independently
authorized post-effect recovery and archival, real core-security account/device
handoff and managed Victoria Operator/policy/ingestion acceptance. Old browser
records remain unowned, retained and export-only, never automatically replayed.
The physical iPhone pasted-image caret issue remains unrelated and unsolved.
