# Typed installation receipt readers: 2026-09-14

Controller, Machine and Web reader source
`b86032f63621a5bdf9679c10f02645344eb89a59` is published and active on Hawk.
Its runtime bridge is `2f7fa2375b753d7503a1102dd415304ad21a9199`; the descendant
changes only the telemetry conformance harness to retain protocols 18 and 19.
The component activator requires fresh main ancestry, so the regular component
artifacts were rebuilt from that descendant before activation. Cold bootstrap
continues to use the accepted runtime revision.

All fresh Service installations, including the old generic path, are paused.
Machine protocol-19 attempt writing is also disabled. This accepts the actual
reader floor, **not** connected installation execution, writer cutover, independent
restoration or the [whole refactor](../plugin-refactor-completion.md).

## Implemented and checked

The [Service bridge](../plugin-install-journal.md) persists the exact observed
Machine target, hashes the complete actor-bound intent into its step and commits
the full typed Machine receipt atomically with its Service phase. Retained
schema-one bytes remain readable, but cannot acquire a new execution grant.
The [Machine reader/executor](../plugin-machine-install-attempts.md) retains
pending evidence without replay, binds original connection/deadline authority,
checks installation incarnation CAS and shares the existing installer across
capability kinds. No parallel Plugin lifecycle was added.

Runtime `2f7fa237` passed the complete pinned `just check-compact`: 1,162 Rust
library, 285 standalone Machine, 1,442 Web and 17 PostgreSQL tests, plus the
formatting, Clippy, dependency/composition, feature and build gates. Platform
ignored tests are not included. The subsequent protocol-harness repair passed
its focused regression and canonical Clippy; no additional full-suite result
is inferred from that one test.

The rebuilt immutable readers and actual cold outputs passed all seven gates:

| Gate | Checks |
| --- | --- |
| Service schema-one/schema-two installation readers | 168 |
| Machine installation attempt readers | 72 |
| Telemetry populated readers | 96 |
| Telemetry independent writer policy | 294 |
| Telemetry startup/local recording | 78 |
| Connected telemetry/fault flows | 45 |
| Real disposable Victoria database pairs | 9 |

All 762 checks passed. Their exact artifact manifests and executable-chain bytes
were independently verified. Installation gates include two real opens of each
populated, absent or invalid fixture; telemetry gates do not prove installation
execution. The final captured actual active/next-transaction recovery/cold matrix
matches this accepted matrix exactly. Historical `previousRelease` fields remain
history, not the next transaction's recovery selection.

## Activated artifacts

| Component / role | Immutable release |
| --- | --- |
| Controller active / next recovery | `/nix/store/r7hbjdnp82sj6g6bn9s8y9ijhh9iw75p-cowboy-controller-release` |
| Machine active / next recovery | `/nix/store/5cvs4mjnaakmc9kl7dg5farsw63zlfr2-cowboy-machine-release` |
| Web active | `/nix/store/3p2mv1lxi60kdyw8h84mgzk8026kv6k9-cowboy-web-release` |
| Controller cold | `/nix/store/l4ji30l5pysy8091kwxmz8igsvvpgv95-cowboy-controller-release` |
| Machine cold | `/nix/store/7m0kq3x3hygg5f23l0yg4rcalfh2riw8-cowboy-machine-bootstrap-release` |

Cold artifacts are from Columbus `8e358baba640f361c39f26a49390741cfd3a972e`,
closure `/nix/store/y9gs82a0pki3r30z1k7r1nv1hx08gwng-nixos-system-hawk-26.05.20260731.5b4f72e`.
Only Hawk's recovery input changed. The owned host transaction retained all
component profiles and their processes. NixOS provenance still changes the
system path: AccountsService/polkit restarted and D-Bus reloaded. This was not
a zero-restart host switch; no Cowboy lane caused those restarts.

The transition matrix pairing prior live releases with the new cold artifacts
passed 522 checks; the actual post-host roles additionally repeated all 96 reader
and 78 startup checks.
Controller, Web and Machine then used their separate component transactions.
All four transactions succeeded and report `published=true`:

- Host: `1789378558420489197-8e358baba640`.
- Web: `1789379155594804887-b86032f63621`.
- Controller: `1789379420481844933-b86032f63621`.
- Machine: `1789379493256515448-b86032f63621`.

## Bounded production evidence

An owner-private online SQLite backup passed integrity checking before the
Controller release. Additive migration 23 succeeded and its stored SHA-384
matches the immutable source file exactly. No applied SQL bytes/checksums were
changed. The installation table remains empty and the new Machine attempt
namespace absent; neither absence is used as reader-compatibility proof.

Actual configuration-only preflight passed as the Service/Machine owner.
Managed telemetry writer/background policies remain unconfigured. No destination
policy change, Operator login or production Plugin installation was performed.
The Controller startup explicitly reports installation admission disabled.

The two component transactions replaced Controller/Machine PIDs as intended.
All 13 existing worker PID/start pairs and all three Victoria process identities
were retained, with no new failed system/user units. The actual Controller and
Machine ELF paths match the accepted releases. Machine reports connected at
`worker-240c2080a8bf9eb8968f`; that unchanged generation is not native-resume or
upgraded-worker acceptance.

Local HTTP and public HTTPS match the release bytes for SPA, admin, their entry
scripts and service worker. HTML/SW use no-store; hashed scripts are immutable.
SPA version is `55226f7178ca1af323cd60281970697d`, SW `cowboy-v1685`. Existing
PWAs require a hard reload. Unauthenticated installation history returns 401.

Private create-only evidence is under `/tmp/cowboy-service-install-bridge.fyNuGy`:

- Reader aggregate SHA-256: `6bbf2af38d5f210f1797dc73fee7fef01ae0f6275ef508fa20a9c963fdc56746`.
- Actual reader-role audit: `60af847b2a89f8ef9a3f389aa293cf2294c39bd740f874236ad6c9a5532a8da3`.
- Service install receipt: `e35ed89fbf220aa3342d8a35f9b58c26768befad33e12ca85f9e1f52eba61a1a`.
- Machine install receipt: `dc6fe9be2dd9f441b4bb1b86b5c02fc201bae53d91bedc0fc6a67df614e21fc0`.
- Host audit: `1e35f70d53400d3bfceddb7e2a4901e7110d4b8aa919f192d454c8eb92a44722`.
- Web audit: `72ddaecaf38b9dbca6a4fec5dbbe371d396b535dc3f36ff07531a9b4950def92`.

Next is actual connected installation/fault/restart acceptance using disposable
signed packages and fixture accounts, followed by explicitly scoped writer
admission. Production Plugin installation, post-effect restoration, native/core
security handoff and managed Victoria cutover remain separate acceptance exits.
