# Workspace extensions — 2026-09-29

Status updated 2026-09-30: implementation, full product gates and the
Web/Controller/Machine releases are complete. **GitHub 0.1.0 is published and
ready in the Catalog.** Installation remains a separate Machine operation.
The original rejected host refresh and its evidence below are retained as
history; the publication follow-up at the end records the accepted actual floor.

The [design](../workspace-extensions.md) describes the signed data-only
`workspace_extension` capability, exact Plugin dependencies, borrowed CLI
connection and generic Code / Review interface. This adds workbench resources;
it does not implement Zed's native WASM extension ABI. The first release supports
Linux x86_64 and offers GitHub Pull requests, Issues and Actions. The workbench
entry is **Extensions → Manage**, with a Machine selector and existing Plugin
installation, upgrade and removal operations.

## Implementation and validation

The feature is in `848b4fcf`, followed by immutable-build closure corrections,
the HTTP 204 lifecycle acknowledgment fix in `ff026039`, and the Machine CLI PATH
fix in `a225d618`. Source and these fixes are published to Cowboy `main`.

`just check-compact` passed in the pinned shell: Rust formatting, lint, feature
and dependency checks; frontend type/lint/build; composition, Provider, site and
native-shell contract checks; 1,653 library, 392 standalone Machine, 27 Plugin
SDK, 126 Zed adapter and 1,946 frontend tests, plus 17 disposable PostgreSQL
tests. This is the complete gate before the final small follow-up fixes.
The lifecycle acknowledgment change subsequently passed frontend type/lint and
four extension API tests, including two new tests for empty acknowledgments and
failed mutation behavior. The real Firefox workbench suite passed three checks;
its fixture digest is
`6fb18a309d71a2334e83b57144af1c746e5b5bf2d9f7bcd1f9ec721544e21fde`.

Signed temporary lifecycle tests exercise exact dependency mismatch, cycles,
installation, removal and reactivation. Runtime tests cover bounded CLI
execution, remote normalization and safe resource projection. Six Agent runtime
recipes and probes passed, including the private Codex adapter's 750 tests.
Its initial inherited `COWBOY_PRIVATE_CODEX_ARGUMENTS` test-environment conflict
was removed from the build environment, without changing login/configuration.

Read-only checks use the actual running Machine's filtered environment and its
packaged GitHub CLI 2.92.0. The existing CLI session can read
`dravengarden/cowboy`. All three declared list endpoints succeed; this repository
has no PR/issue items in the observation, while 50 Actions items and one Actions
detail were checked. Credentials and login state were not modified, and resource
content was not retained in the evidence. This is CLI/API compatibility evidence,
not an installed production extension route test.

## Component release receipts

All three component transactions succeeded and were already published at
dispatch. Web serves service worker `cowboy-v1769`; installed PWAs need a hard
reload to pick up the new bundle.

| Lane | Source | Transaction | Immutable output |
| --- | --- | --- | --- |
| Web | `ff0260393edeb134b45ed3efdeeca6dd66e74de6` | `1790688573040542624-ff0260393ede` | `/nix/store/3ijnlhgrb8n4g1j1xb45x0a6frg13cpc-cowboy-web-release` |
| Controller | `ff0260393edeb134b45ed3efdeeca6dd66e74de6` | `1790688541885275274-ff0260393ede` | `/nix/store/g9skq5qipmiszr6d3dacs5k0x6xiykaa-cowboy-controller-release` |
| Machine | `a225d618a9bf60fd71d2a2a8cbab73c70711914a` | `1790689285372846290-a225d618a9bf` | `/nix/store/6y3vl5pzqad6kafal0ya453jwg11nib0-cowboy-machine-release` |

Local/public health, version and Web cache headers were checked. Machine is
online with generation `worker-dd872931e193f388490f`. All eight existing Plugin
installations remain unchanged. During the first component rollout, 17 of 18
worker PID/start identities were preserved; worker
`cowboy-worker-5e3ad1bacf3c-sess-1789954176241.service` changed PID from 689786 to
1297047 under its original native resume identity and original Codex 3.1.23
Plugin/authentication generation. Its trigger and a subsequent user turn were
not independently verified: this is **not an uninterrupted-worker claim**.
The subsequent CLI PATH Machine release caused no further worker changes.

The shared SDK closure required independently versioned Agent packages before
Controller activation. Claude Code 3.1.34, Claude DeepSeek 3.1.24, Codex 3.1.28,
Codex DeepSeek 3.1.24, Gemini 3.1.24 and Grok 3.1.25 were signed and published to
both Catalog roots. Exact embedded Provider coverage passed; 24 distinct public
artifact URLs were hash-verified using the public HTTPS authority with loopback
origin routing. None was installed or upgraded on a Machine by this task.

## New-kind reader floor

Outer release schema 3 lets old Catalog readers skip the new capability before
decoding its payload. Before component activation, actual active, next-recovery,
cold and candidate Controllers read the complete temporary Catalog with seven
exact signed candidates twice from both Catalog roots: 16 reads passed. Old
readers skip GitHub; new readers accept it. This proves publication-envelope
compatibility, not new-kind installation-journal compatibility.

Service installation conformance passes **174 checks** against actual active /
next-recovery Controller `ff026039` and the prospective cold Controller below.
Machine installation conformance passes **84 checks** against actual active /
next-recovery Machine `a225d618` and the prospective cold Machine below.
These are immutable real readers opening disposable state twice per fixture.

The original 78-check Machine receipt is retained as rejected: its six failures
were the old `Unknown` fixture expecting a slot fence after a completed staging
failure. Existing runtime semantics intentionally permit a fresh authorized
attempt before the durable activation boundary. Test-only `718fb94a` splits
retryable staging failure from uncertain activation; both are now checked and
the latter still requires the fence. No runtime recovery policy was weakened.
The final reader receipt runs against the same immutable runtime artifacts.

Columbus `fa9e9513f839410a0d065dc6a5336babe4362b49`, published to its own `main`,
changes only Hawk's Cowboy input, its aggregate lock node and the exact cold
source assertion. The input pins `a225d618` with a shallow Git fetch; source bytes
are locked by NAR hash. Its complete `just verify` and clean owned
`machines/justfile build hawk` pass, producing:

`/nix/store/h7ai89q3r0k5paiw1dg6dz9gk7mw6346-nixos-system-hawk-26.05.20260731.5b4f72e`.

| Prospective cold role | Immutable output |
| --- | --- |
| Controller | `/nix/store/3kxsl4qp19c0jkiy90nicafjvhznfx3r-cowboy-controller-release` |
| Machine | `/nix/store/vcq7ank4k305my135jv6mhzb3ikf4cii-cowboy-machine-bootstrap-release` |
| Web | `/nix/store/dmalap1fgl50yvj3ah98h4chx1m12shg-cowboy-web-release` |

The complete unit review includes derived AccountsService, D-Bus, polkit and
manual-cache changes. The Machine unit also inherits main's Matrix workspace
registration and retains both lifecycle-preservation flags. This is not a
claim that only one system symlink changes during a successful NixOS switch.

## Outstanding host blocker and publication

Owned activation transaction **`1790690414184684063-fa9e9513f839`** was rejected
at **2026-09-29T22:00:14+08:00**, before mutation. The existing failed baseline is:

- `liveview-backup.service`: derivative object
  `6b967c9294796d2ac837bc7282c3ea39e778594b187a639b35ea36c07aa720cb.tail.op16c`
  has no base object, so the backup integrity check refuses publication.
- `stormbird-cowboy-tail-e43a81f7b4af.service`: earlier transient test timed out.
- `stormbird-wan-e43a81f7b4af.service` and
  `stormbird-wan-fixed-e43a81f7b4af.service`: earlier transient tests exited 143.

The owning [deployment contract](https://github.com/dravengarden/columbus/blob/fa9e9513f839410a0d065dc6a5336babe4362b49/machines/docs/nixos-deployment.md#mechanical-guarantees)
refuses an unchanged failed unit and forbids clearing failure state merely to
pass activation. Repairing unrelated LiveView data and retiring another task's
network-test units require a separate scope decision. The user has been asked
whether to include those repairs; none was attempted here.

The actual host remains Columbus `eea63419` with cold Cowboy `94382b94`.
Before/after rejection snapshots are byte-identical: Controller, Machine, all
18 workers, component profiles, installed Plugins and workspace identity are
unchanged. **The prospective cold-reader checks must not be reported as an
accepted actual host floor.**

GitHub 0.1.0's signed package is retained in `dist/plugins/github/`:

- Package digest:
  `sha256:78b3e25a88720e51d314b925e871818da88cca6c65aab9ed114925c740fd9ee7`.
- Composite artifact digest:
  `sha256:fde8688441460ef9a72c2535546c06f9f8bec8ad4c922f1856626dc524591d4c`.
- Release schema 3, Plugin SDK 1.9, workspace payload 1, no executable matrix,
  no host bundle and no artificial Zed dependency.

After the unrelated host failures are resolved, use the owned host transaction,
bind the accepted reader matrices to the actual activated cold outputs, recheck
the complete exact Catalog, publish GitHub through `just plugin-publish` to
both existing Catalog roots, refresh and verify the exact public artifact and
Catalog compatibility. Do not substitute a component profile for the cold host
floor. Machine installation remains a separate exact-version user operation
under the [canonical release skill](../../.agents/skills/release-cowboy-plugin/SKILL.md).

## Evidence and limits

Bounded evidence and logs are retained at
`/home/draven/tmp/cowboy-workspace-extensions-20260929`. No credentials were copied.

| Receipt | SHA-256 |
| --- | --- |
| `catalog-before.json` | `50f95b27c915718b9d3a787a623b3f9ea3016b2b5d59ecdb9b65bb988a3b2f5d` |
| `controller-readers-publish.json` | `896aa12c5d7e367f13bd4071d0a609f4459ef15ae0153192619d01d8938cef7a` |
| `machine-readers-publish.json` (rejected) | `f873f7a491eba961138047705cdb11adc79281ca2fb743246f97b8edee88523f` |
| `machine-readers-accepted.json` | `870f97dde15d12241bb57b99a8214bf4a8c83a0ff2c6081d65eeb7e3cf4f475d` |
| `real-cli-read.json` | `ecc66f52100958a49024d77aafa3b6c1caba42c730b3b653865a8c2a6c556985` |
| `host-rejected.json` | `045aa2888cb06ec949102ece6a5c6f5770b76099714288c1c113d52df0f0feca` |
| `before-host.json` / `after-host.json` | `15f158a5bc1b056616ecc1e50b33b2e13f71612727c4d56ae7125cbc7b606925` |

No physical iPhone, iOS release, installed production GitHub extension, native
Zed marketplace ABI, new telemetry policy or general post-effect rollback
acceptance is claimed. GitHub API reads are confined to repository-scoped GET;
issue creation, PR review/merge and workflow writes are outside payload 1.

## Publication follow-up — 2026-09-30

The actual successful Hawk host transaction now runs Columbus
`593125efec3824880bff6afcaae1405a95b2e99d` at
`/nix/store/i6dc0pzcmdgm3d1h8pcs1alj2bq6ls6m-nixos-system-hawk-26.05.20260731.5b4f72e`.
Its cold Controller and Machine bootstrap are the `a225d618` outputs listed
above. Current Controller `14c77e88` includes the Catalog null-digest repair.
Fresh checks bind the actual active, next-recovery and cold roles: all 174
Controller journal checks, 84 Machine journal checks and 16 complete Catalog
reads pass. This acceptance supersedes the prospective-only reader result;
it does not reinterpret the earlier rejected host transaction.

`just plugin-publish github` published the unchanged signed 0.1.0 bytes to
both `/var/lib/cowboy/plugins/catalog` and `/var/lib/cowboy/plugin-catalog`.
The existing local Operator refreshed the Catalog without rotating delegation.
The exact composite digest above is `ready`, and downloading the package
through the public HTTPS authority with explicit loopback origin routing
reproduces its package SHA-256. No installation was submitted.

Create-only evidence is under `/home/draven/tmp/cowboy-upgrades-20260930`:
`github-controller-readers.json`, `github-machine-readers.json`,
`github-host-floor-binding.json`, `github-catalog-readers.json`,
`github-public-artifacts.json` and `github-catalog-visible.json`.
