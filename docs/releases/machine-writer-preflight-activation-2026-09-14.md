# Machine writer-policy preflight activation: 2026-09-14

Hawk's resident Machine now runs the writer-policy preflight fix. The user
explicitly authorized this Machine maintenance window. Release source
`327b7f9e05706a8ddabb0d1fe496f91fb28e1dee` was clean and published to remote
`main` before activation. Transaction `1789353836295604800-327b7f9e0570`
committed successfully at `2026-09-14T10:44:05+08:00`, with
`maintenance: true` and `published: true`.

This follows the [unactivated candidate record](machine-writer-preflight-2026-09-14.md);
it does not rewrite that earlier result. The final candidate was refreshed from
current `main` because the activator requires the freshly fetched remote main
to be an ancestor of the candidate. Only the release's source metadata changed
from `0d3c4dad`; the actual Machine launcher and ELF resolve to the same paths.
No runtime source, protocol, durable format, SQL migration, signed Plugin,
Provider credential configuration, native ABI or production managed policy
was changed by this maintenance task.

## Exact artifact and acceptance

Activated release:
`/nix/store/w0dszrabai56bar9zsc59m6bbqwvb5bn-cowboy-machine-release`.
Actual Machine ELF SHA-256:
`2c7311087d7f73d14e0584de97bb4e6a51f5b108fa5a7bd0a4f11c4ff011fa67`.
Desired worker generation: `worker-240c2080a8bf9eb8968f`.

The complete quality gate from the preceding candidate remains applicable:
1,110 all-feature Rust library tests, 272 standalone Machine tests, 1,431 Web
tests, 15 isolated PostgreSQL tests and the other checks recorded there. These
are separate, overlapping suites. Runtime inputs are unchanged; the gate was
not rerun merely for release metadata. The prior 100ms telemetry-test scheduling
failure and final same-oracle success remain recorded, not claimed fixed.

The following gates were rerun against this exact release in the pinned shell:

| Gate | Checks | Result |
| --- | --- | --- |
| Direct immutable CLI/startup probes | 31 | Accepted |
| Populated immutable readers | 96 | Accepted |
| Independent writer policies/effects | 294 | Accepted |
| Controller startup/recording | 78 | Accepted |
| Connected Controller × Machine flows | 45 | Accepted |

All nine Controller/Machine role pairs completed all five connected flows,
including 144 managed-delivery rounds and 324 correlated RPC receipts/HTTP
requests. Lost export ACKs do not resend the admitted attempt. These use
temporary authenticated identities, signed fixture installations and an
isolated protocol receiver, not a production Operator or Victoria database.
The independent audit rehashed manifests, launchers and actual executable
chains, checked all required outcomes and bound every artifact to its role.

The planned acceptance matrix assigned the candidate to Machine active and
the then-active
`/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release` to this
transaction's rollback target. After success, both active and the **next
transaction's rollback target** are the new `w0ds…` release; the receipt's
historical `previousRelease` is not that next target. Machine cold remains
`/nix/store/33iv6hv1mkay3v0h0f2klaa3ds0j3a06-cowboy-machine-bootstrap-release`.
Controller active/next rollback remain
`/nix/store/3wbj6ky4kqp5b88fiv7p3h2pxb28lifn-cowboy-controller-release`; cold
remains `/nix/store/y1iw00838a568nci95w0dldw46kh71fi-cowboy-controller-release`.

The configuration-only diagnostic ran as the actual Machine user, with cleared
child environment and an isolated network, against the intended state root.
It returned exactly `writer_policy: {"state":"unconfigured"}` and all six
`not_checked` declarations. It did not open the binding journal, destination
policy, enrollment or Provider state. Ordinary resident startup is separate
from this diagnostic; the report does not grant execution or delivery rights.

## Production transaction and bounded continuity

The stable Columbus recipe delegated to the **deployed**
`/run/current-system/sw/bin/cowboy-release-activate --maintenance`. Its actual
Columbus revision is `1da44eb88883b52a7bff266da0c07ede35193659`, whose Machine
transaction restarts only `cowboy-machine.service`. The legacy standalone
`cowboy-zed-adapter.service` is retired (`LoadState=not-found`); it was not
recreated or started. The stable checkout's newer/different source description
was not substituted for the deployed transaction contract.

The detached `hawk-cowboy-machine-activate.service` completed successfully.
Machine PID/start changed from `110025` / `2971136905262` to `2131179` /
`3036770249940`; its running ELF matches the accepted hash. The Controller's
deployment-health response reports connected, online and active ACP generation
`worker-240c2080a8bf9eb8968f`. This advertises the desired runtime, not that every
existing worker has migrated.

Independent after/settled audits passed. The Controller PID/start, Controller
and Web profiles/receipts, Web root, host closure/source/receipt, `/healthz`,
`/version` and root/SW ETags/cache policy remain unchanged. Root and service
worker requests returned HTTP 200 with `Cache-Control: no-store`. No new failed
system/user unit appeared. Host closure remains
`/nix/store/ki6hs9zi8w14q42rzj2xyksg6hxckxg8-nixos-system-hawk-26.05.20260731.5b4f72e`.
No Controller, Web, NixOS or Falcon activation was performed.

Worker snapshots at `10:39:50`, `10:44:21` and `10:45:24` (+08:00) span 334.785
seconds, ending about 80 seconds after commit. All **13 worker PID/start,
generation and executable identities** remained unchanged and their units
were active/running. Twelve retained `worker-92b35f0665ec33ba60f6`; one already
retained the older `worker-68cbb63c56ecd76acfa5`. No new-generation worker was
observed in these snapshots. Maintenance did not forcibly stop them or submit
synthetic user prompts to trigger a transition.

For 12 workers, bounded lifecycle metadata for the current PID supplies a
native-session ID hash that remains identical across all three observations
and agrees with the resume-argument hash when present. One worker has no
retained matching lifecycle entry, so its native identity is unverified. Only
hashes and closed lifecycle fields were retained, not raw native IDs,
transcripts, process environments, credentials or journals. The broker was not
probed by attaching a replacement Core peer.

This accepts retained worker processes in a bounded window, **not** complete
generation drain, new-generation native resume, Provider child-process
continuity, a subsequent user turn, a credential rollover or continuous
availability between snapshots. An older worker may transition later under the
normal safe-boundary rules; the differing generation was not relabelled.

## Retained evidence and remaining boundary

Private evidence: `/tmp/cowboy-machine-activate.I4BVjS/`, including the exact
build, five gates, intended-root preflight, activation dispatch log, three
successful host/worker captures and independent receipt/after/settled audits.
The initial capture's obsolete Zed assumption, granular Deno `/proc` permission
refusal and unpinned status command's missing `jq` are retained as diagnostic
failures, not product failures. Corrected captures used fresh destinations;
no runtime oracle or timeout was weakened.

| Evidence file | SHA-256 |
| --- | --- |
| `candidate-matrix.json` | `23f589059938dbd670b00d7167c8e4ba22c21c936fd22f8912d0f54d7ced9fe4` |
| `connected.json` | `8a5f2cfd7dcae6e967e66f6e33da7eba7f2ce671881316fd49d4006e59c9f6db` |
| `reader.json` | `eaa94db67a009b853aa5058fff68f965141513721507162b70e5fe804cef2249` |
| `writer.json` | `22988c779a90f2f2171256dd3274c19c0746877c4e24605a5fb2da36a8c6012c` |
| `startup.json` | `2dc77b0658af14e02c1b912ef42bc5f1e92f65d0ed6e9159983411bd0acbbbcd` |
| `probe.json` | `7e91a263f361edda8f7f01724fb42239ec6132fdb16ace434858cf1cb2a2a7a7` |
| `preflight.json` | `3a438a4b947e60265716bca1d2a66a64c6ec143a15e70e2a5cb22dc723f5b02a` |
| `after/machine.receipt.json` | `e350b02ba6a3cd68dcd14bc0c07a221d03dce12f39f17b6700ff4d512878650c` |
| `worker-continuity.json` | `bb22be0893e9f2a9fd6e3d007047edc4a0dee8a25a2e0c557a7d66ff11c80a18` |
| `after-audit.log` | `e6c3ba06e947f4da7015cc99f5ec1e0c800f76c7c6582f3cc3a959990e98b49a` |
| `settled-audit.log` | `72bf78d4b494e1a464b9651461e58ff45de1ebf0852e43011732966bbfbde1ad` |

The production Machine writer policy and managed binding ledger remained
absent before and after activation. This maintenance does not enable managed
Victoria or disable an existing legacy export configuration. Owned complete
Service/Machine policy cutover, fresh actual Operator confirmation, real
Victoria ingestion/query and production failure/restart acceptance remain
separate P2 prerequisites. First managed intent can permanently fence legacy
export; neither a successful diagnostic nor Machine maintenance grants that
cutover. Emitted telemetry remains `NoRestore`. Generic DAG/P3/P4 work and the
whole Plugin redesign are not declared complete.
