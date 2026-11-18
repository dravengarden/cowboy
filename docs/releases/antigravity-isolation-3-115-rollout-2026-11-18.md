# Component release 3.115.0 rollout (Antigravity account isolation) — 2026-11-18

Status: Controller and Web on Hawk, and the Hawk, Falcon and OVH Machines run
Cowboy `c30266fe`. The seven 3.115.0 Agent Provider releases are published.
`antigravity` 0.2.0 is installed on OVH only. No Antigravity account exists or
is admitted yet. macbook-air still runs its earlier Machine and was not
converged.

## Source

- `99916eb0`: 3.115.0 on `main`. It adds per-account OS identities through the
  OVH `cowboy-account-sandbox` daemon, Machine protocol 48, provider-sdk 3.6.0
  `account_sandbox_v1`, account controls and the Antigravity facade
  ([design](../antigravity-real-accounts-design.md), revision 5), plus the
  review e42e fixes.
- `c30266fe`: the `cowboy-workers` pin advanced to `99916eb0` (worker
  generation `worker-2c2684a56ae848b9c354`).

## Gates

| Gate | Result |
| --- | --- |
| `check-compact` at `99916eb0` | Passed |
| Worker pin: Codex and Claude worker, managed Claude, session, logs, connected code | 34, 96, 7, 39, 11 and 38 checks accepted |
| Generation coexistence | codex 3.6.4 → 3.6.5, claude-code 3.20.46 → 3.20.47, grok 3.2.3 → 3.2.4 |
| Catalog readers on an isolated copy, two runs each | The active Controller 629 and rollback 628 read 74 releases and skip the seven 3.115.0 ones. The candidate reads 81 |
| `provider-release-coverage` | Passed, isolated and live |

## Publication

| Plugin | Artifact digest |
| --- | --- |
| claude-code 3.20.47 | `sha256:88efb9a7737e3f87ea77b7663b61ddf3ac2aad32773b29f98beb591b008b5497` |
| claude-deepseek 3.1.65 | `sha256:fbb89bcbb4909ebf815db4de67b0ea1e153923eccee85b43a64ed3fafd2974e3` |
| codex 3.6.5 | `sha256:c6761a9a39ddd6d4ccbdc0cbf0142ad993e89ad64a4ad6df759506fd35b60a1e` |
| codex-deepseek 3.1.65 | `sha256:72d6ec2a9b7c1fb8d1f5f9072da1e523e78bdca289fae14f7033a0852b46af7f` |
| grok 3.2.4 | `sha256:be3e083c4a5530a5246d394b05aeabae6f99e9f78b07704ca10dc51fcfa754b1` |
| deepseek-harness 1.0.23 | `sha256:28d8df192d219e5d0c15fedf788b7a49b4ee11b643411ec29cbc6fe8bdc2ad71` |
| antigravity 0.2.0 | `sha256:cd27dbd01f652057900dcea210fde0d670a18815d94e75ee4085f5fc7e75682c` |

## Activation

| Target | Result |
| --- | --- |
| OVH account sandbox | Columbus `install.sh` with the static daemon built at `99916eb0` (SHA-256 `dba24d45…8374`); socket and service active |
| Hawk Web, Controller | Receipts `succeeded`/`committed` at `c30266fe`; `/healthz` ok |
| Hawk Machine | `.#cowboy-machine-host-release`, `--maintenance`, receipt `succeeded` |
| Falcon Machine | Built on Falcon (same store path as Hawk), receipt `succeeded` |
| OVH Machine | `tools/ovh-machine-host-release.sh --workers` with `.#cowboy-machine-release` of `c30266fe`: 4 idle sessions hibernated, 6 busy retained, accepted |

## Installation and configuration

- Converged Hawk, Falcon and OVH to the 3.115.0 releases. OVH kept its
  undeclared deepseek-harness, machine-browser and matrix Plugins.
- `antigravity` 0.2.0 installed on OVH by explicit operation
  `ovh-antigravity-0-2-0-acceptance` (state `active`).
- Removed the deprecated `accounts.antigravity_admission` from the Hawk
  Service configuration. `accounts.antigravity_admitted` remains empty until
  the test account is added.

## Next

The test account is added on OVH and admitted, then signs in through its
OVH Machine browser for the acceptance in the design's rollout section.
Real accounts follow one at a time after the soak.
