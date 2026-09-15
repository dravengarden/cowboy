# Independent Agent Plugin publications — 2026-09-15

Status: the six exact releases missing from the earlier
[Controller buffer-owner gate](plugin-controller-buffer-owners-2026-09-15.md)
are signed, published and observed by the running Controller. Both declared
platforms passed their runtime gates. This task performed **no Plugin install,
upgrade, login, credential refresh or component activation**.

This closes that six-release publication prerequisite, not the entire Plugin
refactor or authenticated UI acceptance. A concurrent later change introduces
Claude Code `3.1.23`; that separate release is not covered by `3.1.22` evidence.

## Exact publication identities

All six were independently built from clean, committed source
`530490bcef763c7a35d6b7dc2f15d0ae6ac69124`, with component release `3.8.0`.
Each has its own schema-two envelope, bound host bundle and signature. The
publisher is `cowboy-first-party`; the existing Ed25519 signing identity and
independently selected trusted Catalog public key have fingerprint
`SHA256:a/VJzmHD/94vMVQMNZTktSR9P3apkkKnnWzXrtn02hg`.
No signing identity was created or changed.

SHA-256 values below omit only their `sha256:` prefix.

| Plugin | Previous published version → this release | Composite artifact digest |
| --- | --- | --- |
| `claude-code` | `3.1.20` → `3.1.22` | `80fd0bc200e4149ade2d44f3d6ac79e444a601ac521ad371ff61f7c1feb2e578` |
| `claude-deepseek` | `3.1.17` → `3.1.18` | `05fa79efbd8c59572c1f7b4cd1b19ab3c38695d80899c485c11203d7aab346dc` |
| `codex` | `3.1.20` → `3.1.21` | `b3d9ac50b999b753bdb09a635db3853c3220e9e91bde2b9a3d66708114b0be5e` |
| `codex-deepseek` | `3.1.17` → `3.1.18` | `b5edb2b3d0a316021255af4d082544208692cc7f83c6982450c3fbd16c219740` |
| `gemini` | `3.1.17` → `3.1.18` | `05947b27aea1148a5a0a039bbc3a6f3a3407362c48181fdf82524218e1d3a6f1` |
| `grok` | `3.1.18` → `3.1.19` | `eeceff3c15f66b78c4ee68c9c908b38e485a4d2463c9c678fdd1976544fddb48` |

| Plugin | Package digest | Plugin contract fingerprint |
| --- | --- | --- |
| `claude-code` | `574111cb16161a53711424eb761eee1ef8517c038faf1093f38486317762c5ae` | `f3c426f35ec0e7716eaefcb4673bb7e44a50baf0084cc7fb6e5eac3b3db54871` |
| `claude-deepseek` | `6019464679bcf0d53794ce4b0edcd1a1acc62c0a70a62e6b2201b6033fe35189` | `c375956b6f6c34ee33d5ae90bf4604b71e036d1c7e37485ef599d1d70631f351` |
| `codex` | `895688e445a2e6fceda1db0b10bc05dba32bd2c7aa39369802b5f24e9c43ed5b` | `1bd7d4c34463a331563bed0cda4a982999f45a1053b62a6e267404cc56d1c767` |
| `codex-deepseek` | `e4a3ab35f27681a584ad1dd97eb91258ca34b9e9ec769a00887608596def561d` | `0912c343387e6da116774f9622c53b0fac927816ded8ddcaae85764e0df50239` |
| `gemini` | `2d9ad025222c601943245d00e87f0bf1105cc2b1220b9bc48019fbae58ca0b9c` | `ed9c39314e0eece38a69cb5e11a24d7fd2c59b646ebd173223c9029f734a6a4e` |
| `grok` | `bedccecc53e691334de9af4b5287b7fd61a316bc5045f7a1af9f99f6d0969fdb` | `3209fd48fae6632f4bbcfc8fa09ba37412d1171897a544cffb9cabffb66479e9` |

The supported matrix for every release is **Linux x86_64 and macOS aarch64**.
Public contract components are Plugin contract/SDK `1.8.0`, Plugin API `1.1.1`,
Provider SDK `3.1.10`, Provider UI `3.1.13` and Provider runtime `1.1.3`.

## Dependency and authentication audit

The canonical registry audit at `2026-09-15T14:22:34Z` found every npm pin
already current, with matching official source and integrity. Both private
gateway source trees were unchanged between their exact Columbus pin and
fresh `origin/main` (`5f38ce9d`). This is a no-upgrade audit, not a new upstream
dependency release. All dependency declarations and authentication fingerprints
match the corresponding previous published packages.

| Plugin | Old and new private dependency versions, unchanged |
| --- | --- |
| `claude-code` | Claude Code `2.1.272`; Claude ACP `0.77.0` |
| `claude-deepseek` | Same Claude versions; private DeepSeek gateway `0.1.0` |
| `codex` | Codex CLI `0.154.0`; Codex ACP `1.11.0` |
| `codex-deepseek` | Same Codex versions; private DeepSeek gateway `0.2.0` |
| `gemini` | Gemini CLI `0.59.0` |
| `grok` | Grok CLI `1.0.30` |

Bundled Node remains `24.19.0`. Both gateways remain pinned to Columbus
`6a83e9a9b2b4ac3239cf2c66c0821b1d3e43f781`, with source archive SHA-256
`e7eff8a3bd6de2231056f879c4a0fc568e6f6eaa96f8cdac2171f4da2b4aaa14`
for Claude and
`8fbdbab1e77a504dfea9ebc55628f60ce7d037b51a06dd1d5a56d12d4dd46cdd`
for Codex. Runtime closure and lock checks passed without ambient executable
fallback, credential reuse or dependency self-update.

| Plugin | Old and new authentication-contract fingerprint, unchanged |
| --- | --- |
| `claude-code` | `ceb8d6e09bd5d375f085435d23a52c63976cdf165f07a8b6946484207af4ede7` |
| `claude-deepseek` | `2ffe2f697ae91a568eae585cc3995acb4a4080f08431004da88aaf6058bc5d17` |
| `codex` | `7f8d14c66a1796cc59514c5ac3b9e41beac856b3a13af8851447b52afa7eeb70` |
| `codex-deepseek` | `a7b6784231f07487f1ef34e25e8ed571cec4246bddd0e22209049f8e1493714f` |
| `gemini` | `e993ef7612c5ab51929aaab0a0b8837ac74f57dc5a915e6c2ffb64d0b3b5425e` |
| `grok` | `b150dd54708a22a816fa320d96a255e85e67ff50acc68ee06744154c950ad6ee` |

Hermetic authentication/state tests passed in the complete gate. This does not
accept production Provider login, credential projection or a real model turn.

## Accepted gates

`nix develop -c just check-compact` passed from publication source, including
Plugin/Provider checks, strict lint, type checks, feature-isolated builds and
the owned isolated PostgreSQL suite:

- All-feature Rust: **1,317 passed**, 29 explicitly ignored.
- Standalone Machine: **300 passed**, 2 explicitly ignored.
- Core Code adapter: **26 passed**; private Zed adapter: **25 passed**.
- Web: **1,495 passed**; PostgreSQL: **17 passed**.

All six final runtime builds passed. Each Linux release then passed the real
`agent-worker-conformance` gate against its exact previous published release:
component probes, initialize/session-new, distinct-generation coexistence,
stop and descendant drain. The two DeepSeek tests each retained two independent
gateway sidecars, one per generation. The exact worker was
`/nix/store/0kq61qd65vwy8npqkrkf6g639p2qsfdf-cowboy-0.1.0/bin/cowboy-acp-worker`,
SHA-256 `a8a5025ab7ca86c7c9f38f3e1c7c3ed33de4945f0d8defa98ec2b26cbe58c6be`.
These were loopback-only, private-home fixtures with fake auth and no prompts.

All six macOS matrices were actually probed on registered arm64 `macbook-air`,
using the owned artifact-probe entrypoint from a fresh detached Git worktree
at the same `530490bc` source. Only candidate artifacts and data-only manifests
were transferred; no installed runtime, account or native state was changed.
Those probes do not claim macOS session coexistence or real-account acceptance.

Before production publication, four immutable Controllers each read the entire
isolated 78-release Catalog twice and returned identical reports. The checks
were repeated against the real Catalog after publication. All four also passed
configuration-only `serve --check-plugin-hosts` with the actual Service owner's
arguments and environment, including telemetry preflight schema v1 and
unconfigured writer/background policies. No identity/storage was opened and no
private configuration was included in evidence.

| Actual reader role | Source | Immutable Controller release |
| --- | --- | --- |
| Earlier candidate, not activated | `530490bc` | `/nix/store/b8h4nnwl08fr0xk7bagpg0agvfqsxyg8-cowboy-controller-release` |
| Running / next ordinary recovery target | `0fded719` | `/nix/store/262044kbi5k11xz6yvb5lnn2y3lfg8nb-cowboy-controller-release` |
| Retained previous release | `2f2c0403` | `/nix/store/hqxf3ma74h3fnb1h612h2hdnazw7ww7x-cowboy-controller-release` |
| Actual cold closure | `869c269f` | `/nix/store/cc09k6l788mhchy321ckgg0yryb1hg12-cowboy-controller-release` |

The first reader attempt stopped because another task had changed the active
Controller; it did not authorize publication. Fresh role observations were
captured before the accepted checks above. Earlier disposable build attempts
also found a minimal-Bash `compgen` mismatch, a Deno capability restriction and
an interrupted Mac artifact transfer. Corrected attempts passed independently;
failed logs were retained, not relabeled as acceptance.

## Publication and running-Service observation

The canonical `plugin-sign`, independently keyed `plugin-verify` and
`plugin-publish` commands ran separately for every Plugin. Publication committed
six envelopes to `/var/lib/cowboy/plugin-catalog` between
`2026-09-15T15:16:34Z` and `15:16:42Z`. All **259** pre-existing top-level package,
envelope, host-bundle, receipt and trust-key files were byte-for-byte unchanged.
This preservation check did not rehash every historical runtime archive.

Every bound package/runtime URL was fetched over public HTTPS without login:
**24 distinct URLs**, exact SHA-256 and actual byte counts, HTTP 200,
digest-bound ETags and immutable cache headers. The first download check
incorrectly required a Content-Length header on valid HTTP/2 streaming
responses; the accepted fresh run checks actual byte length and rejects any
contradictory header without requiring an optional one.

The existing signed Catalog watcher logged successful projections through
73, 74, 75, 76 and **78** external releases, finishing at
`2026-09-15T15:16:44.503520Z`. All six publication-source requirements passed
`just provider-release-coverage`. The live Service observation is automatic
Catalog adoption, **not authenticated `/api/plugins` UI acceptance**. No Product
API credential was available; no login or refresh endpoint was invoked to
manufacture one. Primary API identities/platform compatibility remain unchecked.

Between **23:13:41 and 23:18:32 +08:00**, the current Controller, Machine,
Victoria processes and all **13** observed workers retained their exact process
identities. Component profiles/receipts, Web root, host units, cold roots and
failed-unit sets were unchanged. Machine remained online on
`worker-3a889de3bf203a2378b8`. Public and loopback health/version plus five served
Web files matched the immutable active Web bytes and expected cache policy;
Web version was `f86aae072d2953e7194bc79cddf3c6be`.

## Reconciliation with concurrent main and deployment

Before these publications, another task activated Controller `0fded719` in
successful committed/published transaction `1789484626455386558-0fded719b428`.
Its actual executable SHA-256 is
`815560c821c334d322a591322532e8f0074030d1ba3d9730afacde0f43d0def1`.
Git ancestry and unchanged buffer-owner source bind it to implementation
`70aecfad`. Thus that implementation is included in the running Controller;
this task did not dispatch an activator or replace the newer Controller/Web
with the old candidate. Inclusion still does not accept native buffer effects.

The task worktree then fast-forwarded to fresh main `4d8a7dab`, preserving its
independent stream, usage and retained-record fixes. Coverage at that revision
correctly still rejects **`claude-code@3.1.23`**, introduced separately by
`232db694`. Its package-owned stream policy requires its own release evidence;
do not use this receipt to declare that newer version published or installed.
This task did not weaken the gate or perform another Controller deployment.

## Evidence and remaining boundaries

Private local evidence is retained under
`/tmp/cowboy-agent-publication-X8V7VthY`: `release-identities.json` contains exact
old/new identities, source/integrity pins, auth fingerprints and both platform
receipts; `publication-audit.json`, `https-audit-2.json`, `continuity-audit.json`,
`catalog-observation.log`, `staged-v2-readers/`, `published-v2-readers/`,
`staged-v2-hosts-2/` and `published-v2-hosts-1/` retain separate gate results.
Permanent per-Plugin publication receipts are in the real Catalog's `receipts/`
directory, keyed by ID, version and complete composite digest. The Mac probe
worktree retains its create-only receipts under `dist/agent-publication-evidence`.

No target platform is blocked for these six release artifacts. Machine
installation/upgrade, real Operator/Provider authentication, actual native
generation transitions, Review buffer/language consumers, independent
post-effect recovery and managed Victoria cutover remain separate. Follow the
[completion ledger](../plugin-refactor-completion.md); this publication is not
a whole-refactor completion claim.
