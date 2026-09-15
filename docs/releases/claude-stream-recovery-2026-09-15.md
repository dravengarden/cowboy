# Claude partial-stream recovery — 2026-09-15

Status: the Web correction is live and Claude Code 3.1.23 is published, with
Linux worker, actual macOS and production-reader verification accepted.
**Hawk installation and the existing session's idle reload remain pending**;
the reported session still uses its exact 3.1.14 generation.

## Change and source

Implementation commit: `232db694ad41f00a1d5341be5b5f2e91055c01db`, published to
remote main. See [recovery behavior](../claude-stream-recovery.md).

- A package-owned rule retains a healthy ACP process/native session after
  Claude's partial-response errors. No automatic replay of the failed prompt.
- Failed, cancelled and crashed turns cannot turn unfinished tool cards into
  successful ones. Confirmed results and later actual tool updates survive.
- An exact standalone synthetic diagnostic and its adjacent structured error
  render once. Earlier answer text stays separate; raw diagnostics remain
  available in Details. Persisted history is unchanged.

The observed `server_error` is a synthetic upstream category, not proof of an
HTTP 500 or a particular network hop failing. This is recovery/display work,
not evidence that upstream streams can no longer disconnect.

## Verification

The pinned-shell `just check-compact` gate passed:

| Suite | Result |
| --- | --- |
| Rust all-feature library | 1,319 passed; 29 explicitly ignored |
| Standalone Machine | 300 passed; 2 explicitly ignored |
| Core Code adapter / private Zed adapter | 26 / 25 passed |
| Web | 1,501 passed |
| Owned isolated PostgreSQL | 17 passed separately |

Plugin/component checks, dependency checks, strict Rust lint, type checks,
feature builds and shipped builds also passed. The final Details presentation
received an additional type/lint check and 28 focused Web tests. Existing
non-failing Web lint and yanked `spin` warnings were not changed by this fix.

The new real JSON-RPC fixture drives the production ACP session loop: a
completed tool, incomplete tool, synthetic diagnostic and RPC failure are
followed by another user message. It requires one native-session allocation,
no crashed lifecycle, no automatic replay and a successful continuation. A
separate policy test covers errors with and without visible updates and rejects
unrelated authentication/permission failures.

## Web activation

A concurrent application release activated descendant
`0fded719b428552aab083c6d5ec88ba07102c296`:

- Release: `/nix/store/6l3szfg41aj1div6a5w0xa9advhac31s-cowboy-web-release`.
- Transaction: `1789484460500378281-0fded719b428`.
- Service-worker cache: `cowboy-v1694`.

An ancestry check and exact diffs confirmed that Transcript, derive, crashDetail
and the service worker retain this fix. Live `/healthz`, `/version` and `/sw.js`
returned HTTP 200; the service worker is served with `Cache-Control: no-store`.
This task did not dispatch a Controller or Machine restart. Its initial build
processes were interrupted by the concurrent Controller restart; the Plugin
build was subsequently completed again from clean implementation source.

## Signed Plugin release

| Field | Value |
| --- | --- |
| Plugin | `claude-code@3.1.23` |
| Source | `232db694ad41f00a1d5341be5b5f2e91055c01db` |
| Package SHA-256 | `22294e7280a8f67ff0626dcce269f4549928c0ac8cba3a7f9ea962bbaca8c7de` |
| Composite SHA-256 | `7e4d1102abe03fba9d20fbbe752e7443636d9cb6216f60702c5bb1100a1af293` |
| Host bundle SHA-256 | `4801693944262ef7aed73cf9805056225d46957fd18d432a6c241a3b3d3e7388` |
| Provider contract SHA-256 | `ab0da5e49f30fe543c80e12a1ec374e7cc40a4833cfa8f8f69628542f79e5077` |
| Publisher | `cowboy-first-party` |
| Public-key fingerprint | `SHA256:a/VJzmHD/94vMVQMNZTktSR9P3apkkKnnWzXrtn02hg` |
| Platforms | Linux x86_64; macOS aarch64 |

The current main pins are retained: Claude Code 2.1.272 and Claude Agent ACP
0.77.0. The registry audit matched their exact integrity values. The reported
installed 3.1.14 release instead contains CLI 2.1.231 and adapter 0.63.0.
The native-session/configuration behavior and authentication projection/portable
schemas remain compatible. Both authentication contract fingerprints are
`sha256:ceb8d6e09bd5d375f085435d23a52c63976cdf165f07a8b6946484207af4ede7`.

The actual immutable Hawk worker accepted 3.1.14 and 3.1.23 concurrently in an
isolated network namespace using fake auth and no model prompt. Initialize,
session-new, old-generation drain, candidate survival and candidate descendant
cleanup passed. Worker executable SHA-256:
`a8a5025ab7ca86c7c9f38f3e1c7c3ed33de4945f0d8defa98ec2b26cbe58c6be`.
This is not an actual production-history resume or installation receipt.

The active/next-transaction-recovery Controller (`0fded719`, release
`262044kbi5k11xz6yvb5lnn2y3lfg8nb`) and actual cold Controller (`869c269f`,
release `cc09k6l788mhchy321ckgg0yryb1hg12`) each read the staged candidate twice.
All six reads saw 79 releases, including the exact candidate, without creating
Service state or modifying the 78 published releases. Production role identities
and original Catalog file hashes were rechecked afterward.

## macOS acceptance and publication

The actual arm64 MacBook Air passed both exact 2.1.272/0.77.0 runtime probes
against this release's package and composite identity. The fixture used a
private temporary home, no Service credentials and no inference prompt. Its
receipt was copied back and checked before removing the remote fixture.

The configured SSH relay initially timed out, including a fresh non-multiplexed
connection. Direct SSH to the same stable `air.stormbird.xyz` hostname succeeded
with the existing host identity and allowed the probe to finish. No persistent
SSH/network configuration was changed. That transport observation does not
attribute the earlier model stream interruptions.

The owned `just plugin-publish` independently reverified the signature and
published at **2026-09-15 23:31:53.331 +08:00**. Durable receipt:
`/var/lib/cowboy/plugin-catalog/receipts/claude-code-3.1.23-7e4d1102abe03fba9d20fbbe752e7443636d9cb6216f60702c5bb1100a1af293.json`.
All five public HTTPS package/runtime URLs were fetched without credentials;
their complete contents, byte counts, SHA-256, immutable cache headers and
digest ETags matched the signed publication.

The live Controller's Catalog observer reported `external_releases=79` at
23:31:54.998 +08:00. The explicit refresh endpoint returned HTTP 401 because
this tool session has no admin login; the existing observer completed refresh
independently. An authenticated UI compatibility report is not claimed.

## Remaining installation boundary

The [release-cowboy-plugin skill](../../.agents/skills/release-cowboy-plugin/SKILL.md)
states: "do not call a Machine installation or upgrade endpoint unless the
user separately asks to install it on a specific Machine." The installation
route also requires an authenticated Operator; a publication signature or
filesystem access cannot supply that authority.

Hawk's active Claude Plugin and the reported session still identify 3.1.14. An
authorized installation and idle-session Reload with
`Load installed Provider 3.1.14 → 3.1.23` are required for that session to adopt
the recovery rule. This retains the native conversation; an active turn must
not be interrupted. No Machine install or session-reload request was sent by
this task.

The signed release is retained under this task worktree's
`dist/plugins/claude-code/`. Local build logs and accepted Linux/reader receipts
are under `/home/draven/tmp/cowboy-stream-diagnosis-20260915/release/`. No
production credential was used in the fixtures or copied into these receipts.
