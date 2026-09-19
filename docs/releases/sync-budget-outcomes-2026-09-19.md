# Original-operation Budget outcomes — 2026-09-19

Status: typed reconciliation is accepted across the supplied four-process chain;
the **Controller and Web readers are published and active on Hawk**. Zed
`1.18.0` and the new Machine are verified candidates, **not production Plugin
publication, installation or Machine activation**. This is a finite continuation,
not completion of the Plugin refactor or independent post-effect restoration.

Implementation: `f758f9cfa75e736a1c76bedf5a9498a6cbbd4a09`, on freshly fetched
remote `main` `5bc98e05`. Final artifact/harness source:
`17444b806db04f8eb5b085ac020287289552759c`. The latter adds only the shared
Budget fixture to the Controller Nix source closure and its positive/negative
source-boundary checks. Both have Zed subtree
`c52e43d76d11c9fbb22079d9b2cb51e3b9e6dd2b`. Acceptance documentation follows
activation; it is not relabelled as the built revision.

## Behavior and compatibility

The [Budget outcome contract](../plugin-sync-budget-outcomes.md) projects only
an exact original native operation's terminal `refused/budget` result. Local
timeouts/capacity errors, foreign operations and partial replies retain Unknown
and the existing exclusion. Apply remains single-use. Original-ID Query can
reconcile the lost reply; duplicate Apply cannot perform another effect.
Retirement remains a separate explicit operation, not buffer release or undo.
The Web surface names the native resource limit and offers no automatic retry.

Zed Plugin/private adapter advances `1.17.0` → `1.18.0`. Private server `1.4.0`,
upstream `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45`, third-party pins,
Plugin component release `3.11.0`, SDK `1.8.1`, Code schema 2, adapter API 1,
Machine protocol 21 and HTTP API 1 are unchanged. Older closed readers reject
Budget and retain uncertainty; they do not reinterpret it as Source or success.
Controller/Web deployment cannot adopt old native owners or provide this new
outcome through an older Machine/adapter. Update those readers before installing
the candidate in a separately authorized maintenance/release workflow.

The integrated remote offline-first app-shell source lacked its component
version record. This change preserves that implementation and every historical
record, advances `cowboy.app-shell` to `1.1.3`, and appends component release
`3.12.0` with its exact digest. No applied SQL or historical Plugin release is
rewritten. The PWA service worker advances to `cowboy-v1723`.

## Exact native candidate

| Artifact | Immutable output | Executable SHA-256 |
| --- | --- | --- |
| Adapter `1.18.0` | `/nix/store/6q1xw6phni7jjjlm6dsjwrfq5b7r2y3d-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.18.0` | `a2bf32aaf46277620ee700fff3f874dfe295de92eec422bec53887166ba10ace` |
| Server `1.4.0` | `/nix/store/88hccb0s3csayvay7qwisn755pbscp4y-cowboy-zed-server-x86_64-unknown-linux-musl-1.4.0` | `1bfd5b9556f61545906a0f24b08bdfcb96c194a900fc54b8d96992a012dec283` |

The official package-owned builder verified the exact static pair on clean
`17444b80`; it reuses the previously accepted server bytes. The preceding
43-test native build is historical evidence, not a new native-source run here.
The generic SDK bound the candidate after the complete source gate. Its
production signature remains empty and its HTTPS artifact locations remain
planned URLs, not proof of publication or an installable Catalog release.

| Candidate identity | SHA-256 |
| --- | --- |
| Package | `c4fd0bdd96f326c5c10e154d96c0ea21968fb92e00d7faf0b30bf8a5d71b0d2f` |
| Composite artifact | `92b1078ab8bd8030cda30f32e13c7c7b355ffbd4e9a9dda0a8085c23c0ef89a1` |
| Contract fingerprint | `549778b1f7bba61fefcadc757c16a0f4083bb77a2d58475444c33b66259167d0` |
| Final build receipt file | `9ff971603b111a855bbecad8e6cb1c5632496177d40356436c7db7d32b42b6b7` |
| Runtime matrix file | `2cba1d87f19cc76a99c28d1e4d7248a9b828835dda921c8173b7dc283f785f39` |
| Unsigned release envelope file | `a798afec52301b71d755ae16b00269fe3159c6b6a9d622fb4f4381c23a1df8e8` |

## Gates

All build/test commands used the pinned shell and canonical release skill.
No production credentials, installation or external network enter the native
pair, temporary signed lifecycle or connected fixtures.

- Complete `just check-compact` on implementation source `f758f9cf`: main Rust
  **1,482 passed / 34 explicitly ignored**, bridge 3, standalone Machine
  **370 / 4 ignored**, core Code adapter 26, private adapter **126 / 2 ignored**,
  Web **1,815**, isolated PostgreSQL **18**. Formatting, Clippy, strict types,
  dependencies/components/features and optimized builds passed. Existing Vite
  chunk-size and OpenTelemetry lint warnings were not suppressed.
- Final `17444b80` immutable Controller/Machine/Web builds and Nix source
  boundary passed. This is not a second complete source gate: the only later
  source change is the three-line Nix fixture correction.
- Final exact static pair: **5.65 s**, including actual native Budget refusal,
  preserved original text/version/source, one-use Apply and original Query.
- Final temporary signed installation/uninstall/drain and retained native
  generation lifecycle: **10.03 s**. Fixture cleanup is not product recovery.
- Connected **v6: 19 checks, 170.87 s** on clean `17444b80`. Actual authenticated
  HTTP installation, enrollment and all four supplied processes pass, including
  the 1,025-edit refusal, four discarded real replies through normal 40-second
  deadlines, retained fences, exact original-ID Budget observation, preserved
  native text/disk bytes and independent retirement/release. Receipt has
  `stage=complete`, `failure=null`, `cleanup=true`, `accepted=true`; its package
  and native hashes match the candidate. Historical v5 does not cover Budget.
- Firefox 151.0.1: **9 synchronization**, **24 buffer-owner**, **6 actual Review**
  checks passed. Fixture SHA-256 respectively:
  `33e9034289d3af09d281ad0286bc64e850061fbfe0c4b18277af0c4086b7eba4`,
  `4e7a11c5ac7d212fa57370ad912bca000d784143febfa4d683eece9bd7b2da27`,
  `149db8722dcbdb1855d3ddfcfde1068266e05a7e39776ab7203d2234caa4644d`.
  These browser fixtures do not accept a physical device.

Connected inputs: Controller
`/nix/store/ygyk7dd8r0c92fxmk6a2zb47rrw52ndh-cowboy-controller-release` and
Machine `/nix/store/d8dikxzp5bxs9dkzqj8dy1a06ajnzza0-cowboy-machine-release`.
The latter's `worker-3c1c8899619631af0662` is a supplied candidate, not the
production generation. Core adapter SHA-256:
`38660af687380d672f31bf233452c3ece6c361b06a39252cef33abf11fa4c9df`.
Explicit test-only LSP SHA-256:
`6c7b46d70f87dd96f00ef3837ea35abf1d3b0a6c34c1d326387891173b44d3b3`.
Its synthetic answers are not a shipped dependency or production language
acceptance. Source boundary output:
`/nix/store/xwkx18ci2arlxsmkaanif7kfn46s3asm-cowboy-source-boundary`.

## Production reader activation

Actual active/predecessor and candidate Controller `serve --check-plugin-catalog`
and `serve --check-plugin-hosts` passed as the Service owner with its original
arguments/environment. The bounded reports agree. The required telemetry
preflight schema is present: writer/background configuration remains
`unconfigured`; existing legacy selection remains `not_checked`. These checks
neither open storage nor authorize export. Catalog, policy, SDK and applied
migration sources are unchanged relative to the active Controller.

Both machine-owned transactions were published, successful, committed and
`maintenance=false`:

- Controller `1789780198240368933-17444b806db0`, committed
  `2026-09-19T01:10:12.728228722Z`, activates the connected Controller above.
  Previous/recovery release:
  `/nix/store/16c48qyzmj8c2r8g864wyqzlgmpnif65-cowboy-controller-release`.
  Actual Controller PID changes `2496662` → `1051040`; its exact ELF SHA-256 is
  `2c7b22f04489fde1a0d0899f7bf00ff84fdd690005fd650c3abcff376cae4f18`.
- Web `1789780237922116103-17444b806db0`, committed
  `2026-09-19T01:10:37.975146928Z`, activates
  `/nix/store/4zzxb0jfgnv346ljhx691mjl8k15qhli-cowboy-web-release`.
  Previous release:
  `/nix/store/ic9s5f0ybhp2b4irb62cnkv2hs27xr95-cowboy-web-release`.

Local and public health succeed; HTML, SW and entry JS bytes/ETags match the
immutable Web output. HTML/SW use `no-store`; hashed JS remains immutable.
`/version` is `4bb51ded1747cbc6ffef8a7940736881` (SPA hash, not Git revision).
PWA users still need the explicit Update/reload to consume the new bundle.

The before/after and settled audits retain all **17 non-Controller PID/start/executable
identities** (16 ACP workers and the resident Machine), the three Victoria
PID/start observations, Machine profile/receipt/generation and host closure.
Machine remains online on `worker-89a1025ff0eb11271ede`, release
`/nix/store/lmry3df0ncmf1fb8pmjp75v8p47yzizr-cowboy-machine-release`.
This is process continuity, not acceptance of a new native generation, Provider
turn, ordinary Zed instance or production native Budget effect.
The settled check at `2026-09-19T01:13:35Z` repeats all six Web byte/cache checks;
the actual restarted Controller's Catalog/host reports still match preflight.

## Retained evidence and exclusions

Private evidence: `/tmp/cowboy-sync-budget-apon3i`, including immutable input,
bounded host/activation captures, logs and the unsigned candidate under
`candidate/`. Temporary fixture signatures never become release authority.

| Evidence file | SHA-256 |
| --- | --- |
| `full.log` | `3851c43d3d4d50d3dbce9fac30c99f17d344ce68752421f0a7d866bf63f2c7d3` |
| `build-final-artifacts.log` | `6d01f67e83beda500177f62969a6b5bffb84e59c686db75e69479785f10852de` |
| `runtime-final.log` | `3b08b1cce616ffdf7c95584264c0be17ccfb775e0c982a345b29173cd3e1e07b` |
| `native-pair-final.log` | `d77059895f19f37aaf8ed0fccf555acb18989f1f24fe416128aafb1ac1984c8e` |
| `lifecycle-final.log` | `c12fe3cd99fae95d884bf915f050593ac3e7d7f18de8b93041787a6a2aaee5cc` |
| `connected.json` | `5c3b25a503dc2e12b63b5e1f96c07b6cab876ac8f4e73a90f79eedbcc203e420` |
| `connected.log` | `1c88b46c11b77d0fa7990a3f054d76f5180a5d37b9cd59971e88fb323cd36737` |
| `browser.log` | `e71fa69e67333c5a10b1ec97fcf286757adc5d1f5715a06fd32969634497fce7` |
| `bind.log` | `8e11d58a10ff6acc8127bf26d9eff9eb47f579ca94d93c96a70f7731b6fa0c25` |
| `preactivate-preflight.json` | `5099a4021537670eac746a2344046cb4d3921c2d90858049c48844b0966b082b` |
| `activation-after.json` | `85a143107c63e9b7111778ebaef62778e39cb7e82461b0f51fe50f264512884b` |
| `activation-settled.json` | `65a09e878cb8aedfc83a88840a30a849352ba1cafc367251543d029853edcaa0` |

Development failures remain separate: the initial Nix build lacked the shared
fixture, the first corrected invocation named a check as a package, and the
private host audit needed an explicit ES module and permission to inspect
`/proc`. Successful corrected runs did not weaken any product guard. The browser
log contains its three passing suites followed by the initial source-boundary
failure; the final immutable build log is the source-boundary acceptance.

Still open: general graph/site/state leases, additional Agent authentication and
generation acceptance, all-writer/global native history/snapshot/background
limits, independently authorized post-effect restoration, exact production
Machine/native cutover and supported-device acceptance. See the
[completion ledger](../plugin-refactor-completion.md). No Plugin Catalog bytes,
installation, account, policy, Machine pointer or worker generation changed.
