# Buffered Code read authority — accepted Controller release

This delivers [finite buffered read authority](../plugin-code-read-authority.md),
not completion of the whole Plugin refactor. All eleven filesystem/Git readers
retain original core HTTP state and product credentials, check current Session
visibility before read setup and before returning the complete response, and
retain their original logical/transport scope. Observed logout/revocation,
disable or visibility loss discards bytes, errors, conditional replies and ETags.

Credential continuation is shared with Operator approval; its mutation-role and
automation restrictions are preserved. Viewers and scoped automation can still
read as before. Sender proofs are not replayed, and another cookie/login or
auth-off mode cannot replace an ended credential. No Plugin, SDK, native ABI,
Machine protocol, SQL baseline, journal codec, installation or policy changes.

Implementation: `efc50fd5`. Runtime source, built clean, published and activated:
`e66befc783984bc08c31ad771038e3fda63d7d41`, integrating remote main through
`25e0b30c` and preserving its Lilac Flow work. Acceptance source
`27b972d83668d882d8df4488de7c063c11495f17` differs only in the test-only HTTP
observation helper; it records the actual secondary-login response on a failed
check. That diagnostic change does not require another runtime artifact.

## Accepted evidence

- Complete pinned `just check-compact` on the clean integrated acceptance source:
  **1,537 main Rust / 34 explicitly ignored**, **375 standalone Machine / 4
  ignored**, **26 core Code adapter**, **126 private adapter / 2 ignored**,
  **1,833 Web** and **18 isolated PostgreSQL** tests. Formatting, strict
  lint/types, feature boundaries, dependency/Plugin/native-shell/website checks,
  86 structural link vectors and optimized builds pass. Existing bundle-size
  warnings remain visible; ignored gates were not counted as executions.
- **Seven new source tests** cover original cookie/token revocation, no
  credential fallback, disabled users, role/visibility changes, Viewer and local
  mode, device/automation proof non-replay, pre-read refusal and parked success,
  conditional and error responses. Existing Operator and native-owner gates
  remain enabled.
- Clean immutable Controller build passes **1,154 default-feature Rust tests /
  18 ignored** and **3 bridge tests**.
- Connected **v8** rejects pre-fix Controller `9d20c97e` at
  `original_read_credential_revocation`: the third actual `coreFile` reply
  remains **HTTP 200 JSON** after its original browser logs out.
  `accepted=false, cleanup=true`; this is an isolated negative fixture.
- The supplied corrected immutable Controller passes all **21** connected
  checks in **170.89 s**, with `accepted=true, cleanup=true, failure=null`.
  Exactly **four** core file commands span **three** authenticated connections:
  initial Unicode pages, one held/revoked read and one independent valid login.
  The revoked reply is `401/no-store/no-ETag`; a subsequent revoked request
  dispatches nothing. Original-cursor reconnect/restart refusal and all earlier
  native uninstall/cancellation/lost-reply/no-replay checks remain enabled.
- Actual public Catalog coverage passes for all **six** source Agent Plugins.
  Pre-activation and settled role snapshots each pass **8 cold Catalog reads
  and 4 actual-Service host-policy checks** across all **94** releases.
  Candidate, active, next-transaction recovery and actual cold readers agree.
  Managed writer/background policies stay `unconfigured`; legacy selection
  remains `not_checked`. No private environment was copied into evidence.

The connected gate supplies Machine `7269199e` at
`/nix/store/rg1266170l053dkzb292b2hrlxq8xhyh-cowboy-machine-release`, native
adapter `ng88l6jhmpnk07rxjwmz2s00l6llnwk0` (Zed Plugin `1.20.0`) and server
`i9yxq38r7pzhdkmp4bq8sypihaqlnxpq` (private server `1.6.0`). Receipts bind
their exact executable hashes. Disposable login, enrolled transport and signed
Code installation are genuine; fixture teardown is not production recovery.

## Actual activation and bounded continuity

- Controller:
  `/nix/store/91k8n7l4whms3hi9idw0zi53fqrlkshi-cowboy-controller-release`.
- Executable:
  `/nix/store/xj2nlypwzf7xjzn4pz7cmqm1dhm6x3m9-cowboy-0.1.0/bin/cowboy`.
- SHA-256:
  `372d4be3fcc29ac72b88389e372048bd3634ca8c879a80b397dd4adcd82c4ce5`.
- Transaction: `1789808280188718191-e66befc78398`, committed
  **2026-09-19T08:58:24.334689456Z**,
  `outcome=succeeded, phase=committed, published=true, maintenance=false`.
- Previous Controller: `gg0011rjg731fkxzyrsz5w7afa3f75n6` (`9d20c97e`).
  Actual cold remains `hn2zd44ngda15pz6ki1qdjdw6c7ifmfh` (`94382b94`).

The deployment window **08:57:43.448–08:58:44.585 UTC** retains the resident
Machine and all **16 original worker PID/start/executable identities**, the
worker generation, all eight Plugin installation/auth identities, Web profile
and NixOS generation. Local/public health and version, exact SPA/service-worker
bytes/cache policy, Machine presence and running Controller identity pass.
Web remains the independently activated `ed870bd3` release at
`/nix/store/49ws0s01kh84il32xqqihr4dvghscax0-cowboy-web-release`, with SPA
version `78e5e670596be11d547e59c646957c6c`. This task did not activate Web.

The initial audit refused a one-second shift in all three Victoria `ps lstart`
strings. That wall-clock display is not process identity: same PID/name and
kernel start ticks independently prove all three current processes predate the
original Controller and were not replaced during this window. The failed audit
and subsequent diagnostic attempts are retained, not overwritten; a separate
validator binds the original snapshots and kernel evidence by hash.

**Do not extend the sixteen-worker claim beyond that window.** At
**09:00:02 UTC**, the Machine logs `rolling Provider workers` for
`claude-code`; five workers exit successfully and replacement launches follow.
The later read-only inventory observes its authentication generation changing
**11 → 12**, while version, digest and installation incarnation stay unchanged.
This matches the existing `ApplyProviderAuth` / `RollProvider` mechanism.
This task issued no Provider-auth mutation or roll command. The later snapshot
therefore has **11 original workers retained and 5 replaced**, not uninterrupted
continuity; native resumed-thread correctness is not accepted by these logs.
Machine/Web/host components and Victoria remain unchanged.

Private evidence is under `/tmp/cowboy-code-read-authority.Ep9suIdu`:
`check-final.log`, `build.log`, exact negative/candidate connected receipts,
`coverage.log`, `floor-activation/`, `floor-after/`, original observations,
`victoria-kernel-proof.json`, `window-acceptance.json` and the bounded
`provider-roll.log`. No real credential or private destination policy is
published. This is finite observed authorization, not a continuous principal
epoch, unseen role/disable ABA detection, atomic HTTP delivery, general graph
authority, state reader/writer lease or independent post-effect restoration.
