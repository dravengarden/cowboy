# Native execution rollout, 2026-10-02

Codex native remote execution is activated. New sessions can keep the Agent
runtime and its existing authentication on OVH while using Hawk or Falcon as the
execution environment. Existing conversations retain their placement. Claude
remains the intentional context-integration gap in the
[execution contract](../execution-environments.md).

The [production receipt](native-execution-rollout-2026-10-02.json) records the
exact releases, host transactions, installed Codex identity and recovery
evidence. Cowboy application source is
`ac7f58cb46dcc0114bb1b3e67c117669fd3338b4`; Columbus host source is
`d8a08d147e1c41747f7cd897da86751ec128c775`.

## Activated components

| Scope                              | Accepted transaction or outcome                                   |
| ---------------------------------- | ----------------------------------------------------------------- |
| Hawk host recovery configuration   | `1790883120980278155-d8a08d147e1c`                                |
| Falcon host recovery configuration | `1790883105989988698-d8a08d147e1c`                                |
| Controller                         | `1790883492826910412-ac7f58cb46dc`                                |
| Hawk Machine                       | `1790883540630168142-ac7f58cb46dc`                                |
| Falcon Machine                     | `1790883548824431965-ac7f58cb46dc`                                |
| OVH Machine                        | Owned worker maintenance accepted; both original workers retained |
| Web                                | `1790883857072071321-ac7f58cb46dc`                                |

Both host switches preserved resident processes and component profiles. The
subsequent Machine transactions established `worker-2f2104b90426f41a64f3` on all
three connected Machines. OVH retained both original workers through its host
stop and acceptance, restored `KillMode=control-group` and disarmed its
independent rollback timer.

The Controller and Web health checks pass at the public origin. Web version and
index ETag are `b5e982d05f39ad4de20b51a2fbc7f6ab`; the index serves
`Cache-Control: no-store`. An installed PWA must hard-reload to receive the new
JavaScript; reconnecting its WebSocket alone retains old code.

Nine exited historical transient failures initially blocked the host switches.
After explicit user authorization, their unit definitions, journals and bounded
identities were archived on each host before clearing only those failed markers.
No task was restarted and no network configuration changed. The task's own
frozen-convergence refusal was archived separately. Both the original unfrozen
policy and active hourly timer have now been restored.

## Signed publication and OVH installation

Six exact Agent releases were signed, independently verified and published:
Codex 3.2.0, Claude Code 3.1.38, Claude DeepSeek 3.1.28, Codex DeepSeek 3.1.28,
Gemini 3.1.28 and Grok 3.1.29. All 24 unique public artifact URLs resolved to
their declared SHA-256 values, covering 1,063,082,032 bytes. This task
explicitly installed Codex 3.2.0 on OVH; publication is not a claim that every
other Machine or Plugin was upgraded.

The publication coverage checker initially rejected the genuine schema-four
Codex envelope. The release-tool correction in `6bbe265c` accepts that
Agent-only envelope while retaining host-bundle, artifact and receipt checks.
Regressions cover a tampered bound host, an unbound host, the optional-host
case, a wrong Plugin kind and an unknown future schema. Type checks, tests and
exact six-Provider coverage passed. This changes release tooling, not the
immutable application or signed package bytes.

The first installation, `ovh-codex-3-2-0-native-execution-20261002`, exceeded
the reply deadline and later produced an exact `Unknown/Staging/Expired`
receipt. The official reconciliation recorded that receipt and proved the target
had not changed, then saved resolution
`install-staging-2c1dd552a5260721bc584230a33422b7953cdc4f5d1a030edcbcf6b0ff07ece9`.
No installation was replayed and no live pointer or journal was edited.

The two exact public Linux runtime blobs were then imported with the
[native artifact-cache command](../machine-operations.md#preloading-public-runtime-artifacts).
The fresh authorized operation
`ovh-codex-3-2-0-native-execution-cached-20261002` also exceeded the observer's
wait, but subsequent reconciliation obtained its terminal `Applied` receipt and
completed the Service transaction without replay. The accepted installation
revision is
`installation-8d6d5b37cf040c2e6d37c503f8060d897414e0a22bf4223e200f7cba76d5fcc3`.
Inventory independently reports the exact Codex 3.2.0 digest active, with the
existing authentication generation retained and both replica and materialization
current. No installation reconciliation fence remains.

## Verification and limits

- The full Cowboy and reader-bridge gates passed, including Rust, Web,
  PostgreSQL and release checks. Final integration passed the future-SDK reader
  regressions and 16 publication-conformance harness tests.
- [Native worker](../experiments/execution-worker-2026-10-02.json): 18 checks
  cover the actual packaged ACP entrypoint, cold load, target images,
  cancellation and a 35-second interruption without effect replay.
- [Enrolled sessions](../experiments/execution-session-2026-10-02.json): nine
  checks against the final immutable Controller, Machine and worker, including
  two cold opens with the actual next-transaction recovery reader. The same nine
  checks passed against the
  [built cold Controller](../experiments/execution-session-cold-floor-2026-10-02.json).
- [Catalog publication](../experiments/execution-catalog-readers-2026-10-02.json):
  18 exact role/release checks. Before publication, their active, recovery and
  cold roles were bound to the actual accepted host configuration.
- All six Linux runtime artifacts passed their owned probes and exact final
  worker coexistence checks. The final immutable SDK reproduced the signed
  data-only package bytes.
- The actual Hawk service arguments, environment and `/home/draven` workspace
  root passed the intended Controller configuration preflight with all six newly
  published defaults. OVH is the configured runtime. Telemetry writer and
  managed-background policies remain unconfigured.
- Columbus verification and review passed before the host configuration commit.
  Actual activation and public health receipts are separate from those tests.

Native and session fixtures used disposable identities and scripted model
responses. Production subscription model inference, cross-host latency,
token/turn parity, inline user attachments and native nested agents were not
accepted by this rollout. Claude native remote execution remains disabled.
Cross-built macOS artifacts are not macOS execution evidence, and no iOS
application release was shipped.
