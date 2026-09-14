# Machine writer-policy preflight candidate: 2026-09-14

Status: built and verified candidate, **not activated**. Implementation
`97bc4f18cce1537bd8a61f4fe4343f3a377c0629` and standalone-test gate
`0d3c4dad3582942b7111b49c6b2062cb00a2ee4e` are published to remote `main`.
The latter clean revision is the final immutable release source. Incoming Web
changes through `aa7941d8` were retained; this task did not activate Web.

The [core diagnostic](../machine-telemetry-writer-preflight.md) adds
`--check-telemetry-writer-policy` and rejects initially invalid writer policy
before component, journal or Provider initialization. Two red tests reproduced
the earlier initialization effects. Seven additional tests cover the actual CLI,
all purpose declarations, unsafe/malformed files, a held journal and untouched
unrelated state. Normal journal startup independently reloads its real snapshot.
No protocol, durable schema, SQL migration, signed Plugin, SDK or native ABI
changed, and the diagnostic never creates an execution scope.

## Verification

The final `nix develop -c just check-compact` passed from `0d3c4dad`:
format, Clippy, dependency checks, independent binary feature slices, **1,110
all-feature Rust library tests** (22 explicitly ignored), **272 standalone
Machine tests** (2 explicitly ignored), binary/adapter tests, **1,431 Web tests**,
**15 isolated PostgreSQL tests**, and release builds. These are separate suites,
not a count of disjoint tests. Machine-only tests are now a permanent gate; two
Controller-only test helpers received their correct feature guards, not lint
suppression. Existing Web lint/chunk warnings and the yanked `spin 0.9.8`
dependency warning remain; the advisory gate passed.

The second complete run failed the existing
`deadline_stops_retry_for_every_otlp_signal_without_cancelling_the_first_attempt`
test while waiting for its first HTTP request during parallel Nix builds. That
test gives the initial attempt a 100ms budget. After builds completed, the entire
gate passed with its timeout and assertions unchanged. The earlier failure is
retained; this release does **not** claim to fix that scheduling sensitivity.

The final immutable candidate passed:

| Check | Results | Scope |
| --- | --- | --- |
| Direct CLI diagnostic/startup probes | 31 | Synthetic private files, locked journal, no state changes or external network |
| Populated immutable readers | 96 | Candidate plus retained actual rollback/cold artifacts, two reopens |
| Immutable writer policy/effects | 294 | Independent purposes, private-file refusal, durable effects and duplicate reads |

The supplied matrix places the candidate in Machine `active` **for planned
acceptance only**. Machine `rollback` is actual active
`/nix/store/6ic8c71cby451j299hkhrpvxanapmgcv-cowboy-machine-release`; cold remains
`/nix/store/33iv6hv1mkay3v0h0f2klaa3ds0j3a06-cowboy-machine-bootstrap-release`.
Controller active/rollback is
`/nix/store/3wbj6ky4kqp5b88fiv7p3h2pxb28lifn-cowboy-controller-release`; cold is
`/nix/store/y1iw00838a568nci95w0dldw46kh71fi-cowboy-controller-release`.
The matrix is independently bound to actual profiles and the active closure's
bootstrap outputs, but is **not the activated host matrix**. These two gates do
not accept the candidate's connected delivery, production configuration or P2.

## Artifact and maintenance boundary

Final release:
`/nix/store/s5i61khzzf2jfj7gq4b8vf91q3pwrm87-cowboy-machine-release`.
Actual Machine ELF SHA-256:
`2c7311087d7f73d14e0584de97bb4e6a51f5b108fa5a7bd0a4f11c4ff011fa67`.
The independent audit verifies source manifests, launchers, actual ELF chains,
all required successful checks and protocol-18 observations. Cold bootstrap's
public entry and libexec alias legitimately resolve to the same launcher; both
are checked, not mistaken for two distinct exec steps.

The candidate carries `worker-240c2080a8bf9eb8968f`, whereas the resident Machine
still uses `worker-92b35f0665ec33ba60f6`. The differing generation input is the
prior `77756dfa` addition of Tokio `test-util` under Cargo dev-dependencies; the
current generation function hashes the entire Cargo manifest. This task changed
no worker-generation input, but the complete current-main candidate is **not a
same-generation activation**. Do not manually relabel it, infer zero session
impact, or restart the Machine solely to enable this diagnostic.

The candidate's diagnostic ran as the actual Machine user with cleared child
environment in an isolated network namespace against the intended state root.
Its exact closed report is `writer_policy: {"state":"unconfigured"}`; identity,
binding, installation, destination, Operator and delivery remain unchecked.
It neither reads the legacy destination policy nor accepts Victoria readiness.

Captures at `2026-09-14T09:55:30+08:00` and `10:01:12+08:00` agree on actual
host roles, component receipts and Machine PID/start `110025` /
`2971136905262`. This is a bounded Machine observation, not an all-worker or
native-session continuity acceptance. No component/host activator was called;
no production policy, Provider credential, enrollment or binding was changed.

## Retained evidence and next boundary

Private evidence is retained under `/tmp/cowboy-machine-preflight.OO9x21/`.
The final receipts are create-only; initial red tests, the Service-ID fixture
correction, standalone-test feature failure, timing failure, and disposable
driver/capture corrections remain separate. The initial `97bc4f18` build and
driver probe are not substituted for the final artifact or actual ELF evidence.

| File | SHA-256 |
| --- | --- |
| `reader-final.json` | `1ebeb989e00da5fdf3269d91e6c14c4d52ba0444e00719e5460c0f76ba7b2618` |
| `writer-final.json` | `6116728aa31bbb4963302688385de3f0a10b6640437c01be0bf7c907c46aeff6` |
| `probe-final.json` | `e5cb1dd30fa2d9d1cc7cf1c26f6d6a3c711add6272244625fe8518444e4d5091` |
| `actual-preflight-2.json` | `3a438a4b947e60265716bca1d2a66a64c6ec143a15e70e2a5cb22dc723f5b02a` |
| `candidate-matrix.json` | `2aef337cf9948c7650954c3bc79497d6ba190d389599945bc290ec7c17b8d175` |
| `audit-final.json` | `92136d19c39895d0ce77e1eaa707fd7cb4e2bbbc824391933a39961baf0aaf43` |
| `quality-3.log` | `60721e9b4871070e1175d705b0c9192637caf6c4c3659f32f8695cf61848418d` |

Machine activation remains a separate explicit maintenance decision, with fresh
role/configuration acceptance and session/generation impact review. The owned
managed Victoria policy cutover, actual Operator confirmation, real ingestion
and query, production failure/restart acceptance, and later generic DAG/P3/P4
work remain unfinished. Configuration validity is not authority, and already
emitted telemetry remains `NoRestore`.
