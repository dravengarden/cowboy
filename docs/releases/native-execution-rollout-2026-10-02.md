# Native execution rollout, 2026-10-02

The Codex native execution candidate is committed and published as
`ac7f58cb46dcc0114bb1b3e67c117669fd3338b4`. The production Controller has the
Catalog reader bridge; native remote-session creation is still disabled. Claude
remains the intentional context-integration gap described in the
[execution contract](../execution-environments.md).

## Accepted candidate evidence

- The complete Cowboy gate passed. The reader-bridge gate also passed, including
  Rust, Web, PostgreSQL and release checks. Final integration passed the two
  future-SDK reader tests and 16 publication-conformance harness tests.
- [Native worker](../experiments/execution-worker-2026-10-02.json): 18 checks,
  including the actual packaged ACP entrypoint, cold load, target images,
  cancellation and a 35-second interruption without repeating the effect. The
  receipt now binds the private bridge and packaged adapter hashes.
- [Enrolled sessions](../experiments/execution-session-2026-10-02.json): nine
  checks against the final immutable Controller, Machine and worker, including
  two cold opens with the exact recovery Controller. The same nine checks pass
  with the
  [built cold Controller](../experiments/execution-session-cold-floor-2026-10-02.json).
- [Catalog publication](../experiments/execution-catalog-readers-2026-10-02.json):
  18 role/release checks. The active and recovery bridge safely skip future
  packages; the candidate reads all six. This gate uses fixture signatures; all
  six production release signatures were independently checked with the
  configured publisher public key before staging.
- All six staged Linux runtime artifacts passed their owned probes and exact
  final-worker old/new generation coexistence checks. The final immutable SDK
  reproduced the signed data-only package bytes. Cross-built macOS artifacts are
  not macOS execution evidence.

These fixtures use disposable identities and scripted responses. They do not
establish production subscription inference, cross-host latency, token/turn
parity, inline user attachments or native nested agents.

## Production boundary

Hawk accepted the reader bridge `e2ed4f366db44e48d4c400d784df401cdcb1e0a1`
through the owned Controller transaction `1790879369929151336-e2ed4f366db4`,
committed at `2026-10-01T18:29:44Z`. Its release is
`/nix/store/lrdkqhysyyqn2mq5mkxfh1s7qpm8bxvv-cowboy-controller-release`. Health
passed and Hawk, OVH and Falcon retained their prior worker generations.

Columbus source `d8a08d147e1c41747f7cd897da86751ec128c775` is committed and
published. It pins both cold Machine outputs and Hawk's cold Controller to the
native candidate and provides explicit OVH worker maintenance. Its full gate and
final native review passed. This does not itself activate either host.

Hawk's clean full-system candidate was built as
`/nix/store/qc2f210r0b7mjpkzldwax78aiglyfs17-nixos-system-hawk-26.05.20260731.5b4f72e`.
Only `cowboy.service` differs among system units; no user or network units
change. Controller and Machine retain both lifecycle-preservation flags. The
owned activation refused before switching because seven unrelated, previously
failed Stormbird transient tasks remain in systemd. The Controller and Machine
PIDs stayed unchanged. Falcon has two additional historical transient failures.
All nine have zero main/control PIDs, no queued jobs and no timer triggers.
Their bounded failure records were saved; clearing their state requires separate
authorization, not a deployment-check bypass.

Falcon's native clean full-system build also completed as
`/nix/store/6c8b2n6mi86yqrqlpdvz8fc8hxqaz2ib-nixos-system-falcon-26.05.20260714.8eeec93`.
Only its user `cowboy-machine.service` differs; system and network units are
unchanged. Both Machine lifecycle-preservation flags remain present. No Falcon
activation was dispatched against the known failed-unit baseline.

OVH has the exact immutable closure staged with verified archive SHA-256
`ece33b699d8e462e690fab30fe862a3aebb3b65e7c9584e78022205d1c565970` and
root-owned retention links. The owned maintenance receipt contains the committed
helper and override, original host identity and two worker identities. Combined
systemd syntax validation passed. No rollback timer is armed and no OVH service
or Provider installation has changed.

The final immutable Controller's configuration-only preflight also passed with
the exact built Hawk unit's arguments, working directory and environment,
including `/home/draven` and the OVH runtime default. It used the currently
published Catalog; repeat it after new publication. Telemetry writer and managed
background policies remain unconfigured. This preflight opens no live storage
and establishes no session or credential authority.

Because host activation is waiting, the task removed only its own unchanged
convergence freeze through `cowboy operator unfreeze`. The pre-existing state
was unfrozen. Normal convergence is restored; no new Plugin version was
published during this pause. Reapply a scoped freeze before beginning the
Machine/Plugin cutover.

## Pending activation order

1. Resolve the unrelated host failure-state boundary, then use the owned
   Columbus activation on each host. Verify actual cold outputs, retained
   Controller/Machine PIDs and unchanged component profiles.
2. Freeze convergence and bind the accepted recovery evidence to the actual host
   profiles. Publish all six exact signed releases, verify every public artifact
   digest, and run embedded Provider coverage plus the intended production
   configuration preflight. The actual workspace root is `/home/draven`.
3. Activate the final Controller; explicitly maintain Hawk and Falcon Machines.
   On OVH, refresh the bounded process observations if needed, arm the owned
   rollback timer, run `activate-workers`, verify the reported exact generation
   and accept retained workers or native generation handoffs.
4. Install Codex 3.2.0 on OVH through its exact signed identity and one Operator
   operation ID. Activate Web through the zero-restart component lane, inspect
   all receipts and restore only this task's convergence freeze.

| Component  | Prepared immutable release                                              |
| ---------- | ----------------------------------------------------------------------- |
| Controller | `/nix/store/vd0mva862nz1wxlrgz88d86jdpn7mds4-cowboy-controller-release` |
| Machine    | `/nix/store/ga7nc046x8nb992gy518j58dgjl8z0xi-cowboy-machine-release`    |
| Web        | `/nix/store/z8lm0jpwn86xf8c51ac1rwlmgza53kjg-cowboy-web-release`        |

The exact worker generation is `worker-2f2104b90426f41a64f3`; its executable
SHA-256 is `5ba859926b58695f5239589027a5784416f8ab86efa14804eb08581824c7047a`.
No native-execution Provider release or Machine activation is claimed here.
