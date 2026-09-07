# Plugin completion and retirement — 2026-09-08

This records completed production work, not unconditional closure of every
legacy session or physical-device acceptance. The original extraction and
signed Plugin publication remain in [the handoff](../../PLUGINIZATION-HANDOFF.md)
and [publication record](plugin-publication-2026-09-07.md).

## Published and active

- Cowboy implementation: `7ba2b192dce724e8cd56fa32328c1765cef3f161`, merged and
  pushed to main and the original task branch.
- Columbus integration: `362f62201108bba824b8297026f50f0bf09402cc`, merged and
  pushed to main and its isolated task branch.
- Six Agent Plugins 3.1.14 and Zed 1.2.2, component release 2.7.0, remain
  active on both Hawk and Falcon. All 14 exact signed slots and their Service
  authentication replicas passed final verification. All 25 published URLs
  passed byte/digest, size, ETag and immutable-cache verification. The 50-release
  Catalog and historical signed bytes were not rewritten.
- Catalog-only Host/storage authority remains enabled. Password 1.0.0,
  Passkey 1.0.0 and Cardea 1.2.0 remain the enabled Authentication Hosts.

| Component | Active immutable release | Revision |
| --- | --- | --- |
| Hawk Controller | `/nix/store/k5xk133ma0l5cmyy2zpv1v6p5bc7lsq1-cowboy-controller-release` | `7ba2b192` |
| Hawk/Falcon Machine | `/nix/store/f958qjiggdilmarvfzfpnkgqnpzcv2n1-cowboy-machine-release` | `7ba2b192` |
| Hawk Web, unchanged | `/nix/store/iri5qd3l2ni6kxjjfd5w35dhp8g6rwfa-cowboy-web-release` | `54884f7e` |

Machine worker generation remains `worker-8acc0548415e5404ac08`. The public Web
version is `38812d58a478fe33fecb3e10c2ffa096`, service worker 1636. `/healthz`,
both deployment-health responses, exact SPA/SW bytes and `no-store` headers
passed. No Web rebuild/restart was used for Controller-only changes.

Component transactions:

- Controller: `1788804816839747628-7ba2b192dce7`, succeeded at
  `2026-09-07T18:14:05.079165605Z`.
- Hawk Machine: `1788805436044720097-7ba2b192dce7`, succeeded at
  `2026-09-07T18:24:10.138765191Z`.
- Falcon Machine: `1788805517173569027-7ba2b192dce7`, succeeded at
  `2026-09-07T18:25:28.688602176Z`.

## Completed corrections

`/api/auth/me` shares validated device/automation identity with the normal API
middleware; it does not consume a signed proof nonce twice. Replayed, expired,
revoked or incorrectly bound proofs still fail. The CLI now checks live identity
before declaring cached login ready and uses the owned, locked rotating-refresh
path after a Controller restart invalidates an access grant. Refresh proofs
also bind a configured base URL's actual path. The approved device receives
200 with `no-store`; no cookie or Provider credential was borrowed.

Restored prompts wait for the replacement's option vocabulary and authoritative
configuration replies. Failed settings block the prompt; startup failure keeps
unsent input; Cancel cannot later release held input. Real migrations confirmed
native history and settings for eleven idle sessions: eight Codex and three
Grok. Exact auth generations, native IDs, workspaces, titles, drafts, queues and
preferences were retained. Grok's typed first system-record refresh is recorded
separately from its byte-identical conversation tail. See
[migration acceptance](../plugin-generation-migration.md#completed-compatible-cohort).
No inference prompt was sent; live first-prompt ordering is not inferred from
these no-prompt checks and has separate deterministic protocol coverage.

## Native distribution

Cowboy **0.1.28**, build **20260907174051**, was published from clean Cowboy
`6edd2936cc902c05890df137ac903b24159545f2` through the canonical Mac builder and
Hawk SideStore publisher:

- [SideStore source](https://sidestore.stormbird.xyz/source.json).
- [Published IPA](https://sidestore.stormbird.xyz/apps/Cowboy-0.1.28-20260907174051.ipa).
- Public IPA: 4,040,970 bytes; SHA-256
  `153950c5aed2d72b79d13a91813a817210a955386d0ebc633a0acb1eac5f8331`.
- Unsigned builder input SHA-256:
  `e18a151966c8fbe3fbb130be67b185e87085a98b62261e8e144e95d59ab8a6e6`.

The publisher verified the exact private build receipt and IPA digest before
its canonical version/signature-stripping transaction. All older releases
remain in the source. No external `cowboy-shell` checkout, rsync overlay or
cached static library supplied the build. The publisher did not use Apple
signing credentials or install on a device; SideStore handles user signing.
Free-team builds intentionally use the secure system-Safari login path rather
than pretending to possess Associated Domains entitlements.

## Host retirement and continuity

Both host transactions succeeded on Columbus `362f6220`:

- Hawk: `1788805179239554245-362f62201108`,
  `/nix/store/23m7q9j1vgx8aihpv097izzm4pgcr1nf-nixos-system-hawk-26.05.20260731.5b4f72e`.
- Falcon: `1788805180535982432-362f62201108`,
  `/nix/store/va8gd4z7kxhyf6x4ifz3lz1nlgpag2bh-nixos-system-falcon-26.05.20260714.8eeec93`.

The old host-global Zed adapters had zero worktrees before retirement. Their
Nix units, Machine socket arguments, asset-copy bootstrap and component
activator/health dependencies are gone. Both units now report not-found,
inactive, PID 0. Installed Zed owns its exact adapter/server generation. Old
state directories and archived binaries remain untouched for recovery.
Hawk's unversioned startup npm bootstrap was already removed by the preceding
`d82d576d` host activation; retained legacy Agent launch state was not deleted.

The new public deployment-health surface exposes only workspace revision and
a SHA-256 of sorted compact-JSON IDs, not private paths or project names.
Both Machines reported the candidate revision and fingerprint
`d7945e044286a3139266ab9abe69d4e07fb49edbb083f47de8487d6e72ad7d8f`.
Falcon's retired authenticated-inventory health dependency is no longer used.

Host activation retained Controller PID 1886588, Hawk Machine 1777266 and
Falcon Machine 4021147. Independent gateway PIDs were also retained: Hawk
1074951/1020398, Falcon 2711250/2711398. Their missing retention flags were
fixed so pending gateway unit environment changes cannot restart them during
a Cowboy host switch; those changes await their own maintenance boundary.
Explicit subsequent Machine maintenance produced Hawk PID 1944409 and Falcon
4094387 with no old Zed socket argument. No detached agent turn was force-stopped.

The final Machine maintenance required fresh main provenance. Reusing the
already-active `6edd2936` release was correctly rejected; both targets built
the clean `7ba2b192` release rather than bypassing the ancestry gate.

## Gates and evidence

Cowboy `nix develop -c just check-compact` passed: 696 main Rust tests, 8 marked
ignored, 1170 Web tests, six separately executed isolated PostgreSQL checks,
and the native, Plugin, feature-slice, lint, dependency and production-build
gates. CLI regression tests cover cached success, rejection/refresh/recheck,
403/5xx failure, bounded retries, auth-off and prefixed paths.
Columbus `nix develop -c just verify` passed, followed by clean committed full
NixOS builds on both targets and all owned activation checks.

Create-only private evidence under
`dist/provider-runtime-cache/host-cutover-20260907/`:

| Evidence | SHA-256 |
| --- | --- |
| `completion-gate-20260908-03.log` | `410910c184fe7803b4222338741a225f0670fc85936178a1a9a39d694afaeeef` |
| `artifact-controller-preflight-1788804784236.json` | `c75de61e5ff844c53cc87c9896c8d400d78420dda897c4542854c6bb4d15f136` |
| `artifact-controller-live-1788805649486.json` | `18c5729b9c7efebe041279cb3b76208da40b7256cb52841f5514866c4bdffee6` |
| `public-artifacts-accepted-1788805020543.json` | `5432d83a8f5720439595366e31c7bcc287354487f9fa1527744d60fa65c1a0dc` |
| `device-plugin-accepted-1788806813288.json` | `6ac6c22557426c7a948af8ca908c206c2c2990f03f3a16d9d7eeb17f16ea2a4f` |
| `native-migrations-accepted-1788806812933.json` | `1314c6811c1278e5e64ebd94023c92092c3051b035361ed2a55aa444463d0035` |

Columbus gate log `/tmp/cowboy-columbus-retirement-20260908-03.log` has SHA-256
`5bf4650bf04328c3cd6781c6b10754da311f9bbbed36d0d2f7a8710e1f69bbc7`.
Earlier failed preflights and superseded evidence were retained, not overwritten
or presented as acceptance. The canonical iOS skill passed repository checks;
the generic skill-creator validator's pre-existing unsupported
`disable-model-invocation` frontmatter was not silently removed.

## User-approved legacy cleanup

The user subsequently explicitly authorized discarding the eight unbound
pre-Plugin Codex sessions and their Cowboy history. A fresh visible, idle-only
preflight fenced the exact eight IDs. The same approved Product device issued
only their normal WebSocket `delete_session` commands. All eight now return
404, have durable deletion timestamps, and have no remaining worker unit
(`not-found`, inactive, PID 0). The complete active-ID inventory lost exactly
those eight; the eleven migrated sessions and current session retain their
identities/drafts and active workers. Controller health remained 200.

This uses Cowboy's recoverable three-day deletion window. The eight become
purge-eligible at `2026-09-10T21:50:11.947Z` through
`2026-09-10T21:50:12.695Z` (2026-09-11 05:50 Asia/Shanghai), and the existing
six-hour sweeper subsequently removes their Cowboy event rows. No retention
policy, SQL data or migration checksum was manually changed. Shared native
homes, authentication, project sources and signed Plugin archives were not
deleted. This cleanup closes the legacy-cohort decision, not all compatibility
code or immediate physical-erasure guarantees.

Additional create-only evidence under the same private directory:

| Evidence | SHA-256 |
| --- | --- |
| `legacy-cleanup-before-1788817801456.json` | `082ef3d688aaba6a6d5b0617552255dcf744f0f1a443006f231f91fd2c775f86` |
| `legacy-cleanup-accepted-1788817918522.json` | `091fe55cf18662641d8e594b97e42a9a744cf34dd14e4ddaa06d5b0e4b4d2862` |
| `legacy-session-audit-1788817823932.json` | `8df2f000bf2c13283cc88dcd1793a2360db63e803edcf412d1b7dd3781c65df2` |

The final audit leaves no retained unbound Hawk session. The scoped cleanup
helper passed the pinned Deno type check; no product binary or deployment was
changed for this operation.

## Remaining boundaries

1. Current session `sess-1788279284753` was Busy during the audit. Its 3.1.8
   binding and drafts are intact; use **Load installed Provider** after the turn
   finishes. No background task or active-turn stop was hidden in this release.
2. Physical iPhone/iPad installation, Passkey/OIDC login and input acceptance
   require the user's device. Simulator/IPA publication is not a substitute.
   The known pasted-image caret/IME issue in PITFALLS #69 remains open; no
   composer workaround was shipped or claimed as a fix in this task.
