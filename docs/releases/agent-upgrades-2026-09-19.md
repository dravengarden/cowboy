# Agent publication, Hawk upgrades and Controller activation — 2026-09-19

The five missing Agent publications are now signed, publicly available and
installed on Hawk. The primary Catalog covers all six source Agent versions. The
Controller containing the local Operator lock-release repair is also activated.
A fresh whole-inventory convergence plan has no steps or skips. This closes the
specific publication/activation prerequisite recorded by the
[Zed remote-edit release](native-remote-edit-budgets-2026-09-19.md), not every
[Plugin refactor completion exit](../plugin-refactor-completion.md).

## Exact independently published releases

Packages and both runtime targets were built from clean committed source
`735372fde5a0f9e0273029ed3a8357f028cfbed4`. Each uses component release
`3.11.0`, Plugin SDK `1.8.1`, Provider SDK `3.1.11`, Provider UI `3.1.14`,
release schema 2 and its independently bound host bundle. Platforms remain Linux
x86_64 and macOS aarch64. Provider compatibility ranges remain Controller
contract 2 and Machine contract 4; the observed Machine transport is
protocol 21.

| Plugin            | Hawk upgrade      | Composite SHA-256                                                  |
| ----------------- | ----------------- | ------------------------------------------------------------------ |
| `claude-deepseek` | `3.1.18 → 3.1.19` | `21513a5a78f0af4276251ebbf5a4cd2bc80e4dc916691293f4747c58bdac7a3c` |
| `codex`           | `3.1.21 → 3.1.22` | `aef6cb16919c692b3c6a2f3de2293c45938ef33dcf4a2fe2b9b25a5536948a01` |
| `codex-deepseek`  | `3.1.18 → 3.1.19` | `94dca05c843f39b479eda0d448ef47682803b2319ccdfe0fef460969222266d2` |
| `gemini`          | `3.1.18 → 3.1.19` | `f78bfa05ccad4bb193a00f8f49e68fe728f7019bc762b7785df3cdb2588155a9` |
| `grok`            | `3.1.19 → 3.1.20` | `2d413b8ec099235ca7622bbedb044a336dc0b9eb7fbdc49bc1716165ffa4bc8f` |

The unchanged publisher is `cowboy-first-party`, Ed25519 fingerprint
`SHA256:a/VJzmHD/94vMVQMNZTktSR9P3apkkKnnWzXrtn02hg`. The canonical signer and
independent immutable SDK verifier check package, release and host bundle.
Publication uses `/var/lib/cowboy/plugins/catalog`; durable receipts are
`receipts/<plugin>-<version>-<composite-hex>.json` beneath that root. All 323
pre-existing Catalog metadata/package/receipt/key files across both roots retain
their original hashes. The resulting combined Catalog has 94 releases. All 23
distinct public package/runtime URLs pass full-body SHA-256, byte-count,
immutable-cache and digest-ETag checks. Delegated refresh reports 94 releases.
The [website list](https://dravengarden.github.io/cowboy/plugins.json) already
advertises these exact versions and component releases; website metadata is not
installation authority.

This publishes the previously declared versions, not an upstream dependency
update. Registry audits retain the exact integrity pins for Claude Code
`2.1.272`, Claude ACP `0.77.0`, Codex `0.154.0`, Codex ACP `1.11.0`, Gemini
`0.59.0` and Grok `1.0.30`. Newer registry versions were reported and
deliberately not mixed into this rollout. Both private gateways retain Columbus
source `6a83e9a9b2b4ac3239cf2c66c0821b1d3e43f781`, their source archive digests
and versions `0.1.0` / `0.2.0`.

## Runtime and source acceptance

- `just check-compact` passes on package source `735372fd` and again on
  integrated Controller source `e10bdaf3`: 1,497 main Rust tests, 375 standalone
  Machine tests, 126 private adapter tests, 26 core Code adapter tests, 3 bridge
  tests and 18 isolated PostgreSQL tests. Web tests increase from 1,818 to 1,819
  with the integrated Web change. Existing explicitly ignored tests remain
  ignored; package/provider, auth, formatting, lint, types, boundaries and
  optimized builds pass. Existing dependency/chunk warnings are not suppressed.
- All five Linux old/new-generation gates execute the actual immutable Hawk
  worker, SHA-256
  `8c68e9891fa47a162fd41d68f3ef19edbcc5ce41c229b4ac991912ed0cfd5253`. Each uses
  its exact previously installed signed release, fake credentials and an
  isolated loopback-only namespace. Initialize, session-new, distinct generation
  coexistence, independent gateway ports, old drain with candidate survival and
  final descendant cleanup pass. No inference prompt is sent.
- MacBook Air is offline; configured relay and direct existing-host SSH both
  fail before authentication. **There is no fresh Mac execution or Mac
  install.** The retained
  [2026-09-15 actual arm64 probes](agent-publication-2026-09-15.md) cover these
  exact previous releases. A separate retained-evidence check independently
  verifies their signatures and package hashes, equal complete runtime matrices,
  equal runtime/authentication/launch contracts and platform matrices, and the
  hashes of both rebuilt and previously published Mac archives. All 10 Mac
  components match their original command/version/digest probe identities
  exactly. The original receipts remain unchanged, with separate evidence
  explicitly recording `new_execution: false`. This is reuse for identical
  bytes, not a relabelled new-release execution receipt.
- All five authentication-contract fingerprints are unchanged. Hawk auth
  generations remain Claude DeepSeek 1, Codex 6, Codex DeepSeek 1 and Grok 987;
  Gemini remains without an auth generation. No login, credential copying,
  rotation, Provider policy change or production inference is performed.

## Hawk installation and concurrent Web updates

The canonical convergence dry run selects exactly these five version/digest
pairs. Each dispatch binds its independently verified signed envelope, original
installation revision, enabled admission, no reconciliation requirement and zero
reported session leases. Each has one deterministic operation ID,
`hawk-<plugin>-<new-version-with-hyphens>-converge`. Saved intent precedes the
single normal Operator upgrade. All five return HTTP 204, Service `completed`,
Machine `applied`, matching new installation revisions and retained previous
generation digests. Final read-only observation confirms the same original
receipts and exact installed inventories after the Controller restart.

Another task activates Web `e10bdaf3` after the first two installs. The strict
continuity check fails and the third dispatch is refused before submission; both
failures are retained. Read-only reconciliation proves only Web changed, with
the original 20 Cowboy process identities still present. The remaining three
original intents then complete, without redispatching the first two. No Plugin
install restarts Controller or Machine.

Another Web activation, `cca62b7a`, occurs during the first post-Controller
reader audit. All its individual reads pass, but the end-of-probe profile check
fails; that failed audit is retained. A separate final audit against stable
current roles passes. Both independent Web releases are preserved, not reverted
or attributed to this task.

## Controller activation and final acceptance

An invalid external `--machine` argument is rejected before dispatch. The
correct positional invocation then rejects the original candidate because remote
main has advanced to `e10bdaf3`. Neither rejection changes the active Controller
or starts an activation transaction. After fast-forward integration, a fresh
clean immutable Controller build and the complete source gate pass.

The exact integrated Controller runs all nine local Operator conformance checks
against the actual immutable Machine release, including default denial, private
UID/TCP separation, one effect, revocation and saved-ID/restart non-replay.
Eight Catalog reads and four actual-Service host preflights pass before
activation across active, next-transaction recovery, actual cold and candidate
Controller roles. No private policy/environment is written into evidence.

The machine-owned component activator commits transaction
`1789797398855645314-e10bdaf34fd2` at **2026-09-19 05:57:03.516 UTC**:

- Controller source: `e10bdaf34fd24c940f50700c2915b3f5ae338891`.
- Release:
  `/nix/store/6svq7ihpgxbngmz00sv1nji06zmvwl1s-cowboy-controller-release`.
- Outcome: `succeeded`, phase `committed`, published `true`, maintenance
  `false`.
- Controller PID changes from 2621243 to 2796924; Machine PID remains 2129810.
  All **18 original ACP worker PID/start/executable identities** remain; the
  original Machine and all three Victoria processes are unchanged.
- Machine remains revision `7269199e`, generation `worker-3a3c8b33f96774b1981e`.
  No Machine or NixOS maintenance is dispatched.
- Final Web is the independently deployed `cca62b7a`. Local/public `/healthz`
  and `/version` pass, Machine presence is connected/online, and both served
  HTML and `sw.js` match the active Web output exactly with `no-store` headers.
- The final 94-release reader matrix passes all eight reads and four real-host
  preflights. Managed telemetry writer/background policies remain unconfigured;
  legacy selection remains unchecked. This rollout does not enable telemetry.
- The final whole-inventory convergence dry run has no steps, skips or remaining
  upgrades. Claude Code `3.1.28`, Zed `1.20.0` and Victoria `1.1.0` are
  unchanged.

No existing-session forced migration, production prompt/history-resume test,
physical-device acceptance or fleet-wide upgrade is claimed. The later Web-only
`cca62b7a` source is integrated for this record; it is not a second Controller
deployment or part of the earlier full-source gate.

## Retained evidence

Private root: `/tmp/cowboy-agent-upgrades.Ai1NFK`. Per-Plugin installation
intents, responses and acceptances, both original failure observations,
dependency audits, Linux receipts and distinct retained Mac receipts remain
there.

| Evidence file              | SHA-256                                                            |
| -------------------------- | ------------------------------------------------------------------ |
| `publication-audit.json`   | `34c7fe6b123733b287c23839d9d6e2562a7011c35cac2bdf525917bcbebd3618` |
| `https-audit-1.json`       | `f3436c233fbcfdce31a5bc987224ef0bbd5d354924b18e66cad9755cf136723b` |
| `check-integrated.log`     | `3174fc12bccedb0973191bf5fb3c8bb0edee5c51e563d569089af4df56de791c` |
| `operator-integrated.json` | `551514dc4257ecd3ef64f3d95b69b33821990d7add3985af92563828030eccef` |
| `audit-final.json`         | `571b59488d24a8ae427dd64ddc4e5dbda510faf1e2a203189b09bba90cdb93b6` |
| `final-floor.json`         | `1a7ec5363c851bc33cf766b00a885bd1b00031b219050b39ded3b36d0a82a034` |
| `assets-final.json`        | `b8fbee4fa15dbc5079f7f9b019dcfef82b987961b8670b9049c0bbc698b32913` |
| `plan-activated.json`      | `9ff23b3f48df954228cd8366fba525648e7ea9ce133df2b577ca51f5802cf019` |
