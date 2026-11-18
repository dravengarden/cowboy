# Component release 3.113.0 fleet rollout — 2026-11-18

Status: Controller and Web on Hawk run Cowboy `50d8118b`; all four Machines run
the 3.113.0 Machine; the seven Agent Provider releases are published. The
`antigravity` Provider is installed on Hawk only, for the dedicated test
account, with `accounts.antigravity_admission` enabled for that acceptance.

## Why

Component release 3.113.0 adds the `antigravity` Provider and provider-sdk
3.5.0's `provider.native-custody.v1` capability (see
[`antigravity-provider-plan.md`](../antigravity-provider-plan.md)). The SDK and
wire changes made the `1d90b1b1` worker pin refuse every host-only Machine
release, so the pin advanced with this rollout.

## Source

- `70c38af1`: 3.113.0 on `main` (Antigravity package, native-custody fence,
  human-paced gates, Cargo vendor hash).
- `50d8118b`: `cowboy-workers` pin advanced to `70c38af1` (worker generation
  `worker-1fec2efa29b92198de12`), on top of the third-party upgrade commits
  that landed meanwhile and moved codex to 3.6.3. Those commits change no
  Rust, Cargo or interface file. Evidence:
  [`sdk-3-5-worker-pin-2026-11-18.json`](../experiments/sdk-3-5-worker-pin-2026-11-18.json).

## Gates

| Gate | Result |
| --- | --- |
| `check-compact` at `008a2c8d` (all 3.113.0 source) | Passed |
| Worker pin: Codex / Claude worker, managed Claude, session, logs, connected code | 34 / 96 / 7 / 39 / 11 / 38 checks accepted |
| Generation coexistence | codex 3.6.1 → 3.6.2 and → 3.6.3, claude-code 3.20.44 → 3.20.45, grok 3.2.1 → 3.2.2 |
| Catalog readers on an isolated copy, two runs each | Active Controller 625 (SDK 3.4.0) reads 67 releases and rollback 624 (SDK 3.3.0) reads 61; both skip all seven 3.113.0 releases, including antigravity. The candidate reads 74 |
| `provider-release-coverage` | Passed, isolated and live |

`agent-worker-conformance` cannot run antigravity: the harness extracts each
component into its own flat directory, while `agy-launch` resolves the sibling
`provider_cli-antigravity` component through the Machine's installed layout.
The Hawk installation below exercised that layout instead.

## Publication

Built, bound, signed (`cowboy-first-party-v1`) and verified. Antigravity's
release binds Google's versioned `agy` 1.5.9 URL; publication copied only its
package and the `agy-launch` archive, and the Catalog holds no `agy` bytes.

| Plugin | Artifact digest |
| --- | --- |
| claude-code 3.20.45 | `sha256:6f6393cac82690cf094c249c681920c588f3e2a92878e589ccf534b6e936fd4e` |
| claude-deepseek 3.1.63 | `sha256:0879a38a4aee94ceac99a95df0f2090ff105698d8629edb41fcd7064cc2cb845` |
| codex 3.6.3 | `sha256:4f14b21ff2eb80f5a648b04604d5d46091a287da327048cd86e763b5308355cf` |
| codex-deepseek 3.1.63 | `sha256:e1835f84eb89c43c898ae1d7620b16ed726219e99eae8902350e695dce7fe0c7` |
| grok 3.2.2 | `sha256:96ce948f32d7cc4a1166e777877a270f50c4ebad304e2b8e8d9f01013509c293` |
| deepseek-harness 1.0.21 | `sha256:41b40a590752daba3c28e45fc8d21e8c5cb3a13de680386ea01ef724d411c4ea` |
| antigravity 0.1.0 | `sha256:1871f2758b0d98889baf145015596112cc2ad5fdcff3debbd429ed4d23164f92` |

## Activation

| Target | Result |
| --- | --- |
| Hawk Web, Controller | Receipts `succeeded`/`committed` at `50d8118b`; `/healthz` ok |
| Hawk Machine | `.#cowboy-machine-host-release`, `--maintenance`, receipt `succeeded`, worker `worker-1fec2efa29b92198de12` |
| Falcon Machine | Built on Falcon (same store path as Hawk), receipt `succeeded` |
| OVH Machine | `tools/ovh-machine-host-release.sh --workers` with the plain `.#cowboy-machine-release` of `50d8118b` (worker `worker-293a2fe5d6067490374a`, built from that revision): 1 idle session hibernated, 6 busy retained, accepted |
| macbook-air | Built on the Mac, `cowboy-machine-install --refresh` with its existing arguments; installed hashes match, launcher unchanged, backups suffixed `pre-rollout-3113`; reconnected after its ~75 s startup checks |

## Installation

| Machine | Result |
| --- | --- |
| Hawk, Falcon | claude-deepseek 3.1.63, codex 3.6.3, codex-deepseek 3.1.63, deepseek-harness 1.0.21, grok 3.2.2 |
| Hawk | antigravity 0.1.0 by explicit install (`hawk-antigravity-0-1-0-acceptance`): `agy` downloaded from Google, SHA-256 verified, installed bytes equal the official 1.5.9 executable; `agy-launch --version` from the installed layout prints `1.5.9` and writes nothing; the Machine user's `~/.gemini/config` was not created |
| OVH | claude-code 3.20.45, codex 3.6.3, grok 3.2.2; undeclared deepseek-harness, machine-browser and matrix kept |
| macbook-air | claude-deepseek 3.1.63, codex 3.6.3, codex-deepseek 3.1.63, grok 3.2.2 (bounded with `--plugin`) |

Existing sessions keep their pinned generations until they change release.
