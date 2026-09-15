# Zed operation connection boundary — 2026-09-15

Published and activated on Hawk from clean Cowboy revision
`2f2c0403b839d4dc395ec4fadcfed40d425e435d`. The separate test synchronization
commit `1053a0b2` follows the upstream Plugin-declared account ordering and
Anthropic widget metadata; it changes no Plugin package or Web runtime code.

- Controller release:
  `/nix/store/hqxf3ma74h3fnb1h612h2hdnazw7ww7x-cowboy-controller-release`.
- Executable:
  `/nix/store/liamclvy1dk5cm9zqvmpm42c40pwd9qz-cowboy-0.1.0/bin/cowboy`.
- Executable SHA-256:
  `f2216b650845948721fbb1c6f88e4a6aab21577c245bd636ca1cd70203607f81`.
- Transaction `1789472583452464498-2f2c0403b839`: `succeeded`, `committed`,
  `published=true`.
- Effective predecessor:
  `/nix/store/lvr03vnxgpqjm91dvf4mpif7niqi0r6k-cowboy-controller-release`
  (`c12a2d63`).

## Accepted scope

The [finite Zed operation](../plugin-code-read-scopes.md#zed-operation-connection-lifetime)
retains its original Session observation and authenticated Machine connection,
or one connected local Unix peer. Worktree readiness and buffer open cannot
cross a same-epoch reconnect or resolve a replacement socket pathname between
calls. The command registry checks the original connection at enqueue and after
the reply reaches its caller. Failure and cancellation end the local operation
without inventing a retry, cleanup RPC or native restoration.

Opening requires an API-1 worktree response whose state is exactly `ready`.
Non-ready states and wrong variants cannot open a buffer and return the existing
HTTP `503` readiness error. Local socket connects have a two-second deadline;
each combined write/read exchange has a 35-second deadline and 4 MiB request and
newline-framed response bounds. EOF without the newline is incomplete.

This is not cross-request buffer ownership. The old close protocol still uses a
browser lease ID and current Session target; it does not carry an original
core-owned resource handle. Retargeting, deleted files or lost open replies can
leave native resources uncollected. Connection cleanup is not compensation for
an already-dispatched native effect. General graph/state resolution and exact
native-generation release/recovery remain separate work.

No Plugin source, SDK, Machine protocol, journal format, migration, host policy,
native ABI or production Web bundle was changed by this repair. The existing
unbound adapter callers keep their behavior.

## Verification

Two regression tests failed before the repair: completed readiness followed by
same-epoch reconnect allowed buffer open on the replacement channel, and
non-ready worktree states still opened buffers. Sixteen new source tests cover
these boundaries, normal open/close payloads, foreign registries, late replies,
pending-waiter isolation, cancellation, response framing and byte limits,
local read/write deadlines and real Unix socket pathname replacement. The Zed
suite passed 17 tests including its five existing cases; four additional tests
exercise the Machine command registry.

The integrated `nix develop -c just check-compact` gate passed: format, strict
Rust lint, dependency checks, independent feature graphs, composition checks,
tests and shipped builds. Rust passed **1,279** all-feature cases with 29
explicitly ignored; the isolated PostgreSQL gate separately passed **17**
cases. Web passed **1,481** unit cases. Earlier full-gate attempts exposed three
upstream stale metadata assertions; these were synchronized with the declared
Plugin values and the entire gate reran successfully. The two changed Web test
files additionally passed 28 targeted cases, type checking and lint.

The immutable Nix Controller build passed. Actual candidate, effective
predecessor and cold Controller binaries read the public Catalog with closed
environments and an absent disposable Service data path. All returned the same
**72 ready release identities**, including all six embedded Agent versions;
no Service state was created. Cold remained
`/nix/store/cc09k6l788mhchy321ckgg0yryb1hg12-cowboy-controller-release`
(`869c269f`).

The publication-coverage gate initially refused missing upstream versions.
Their publication became available during this work; final coverage accepted
`claude-code@3.1.20`, `codex@3.1.20`, `grok@3.1.18` and the three remaining
Agent Plugins at `3.1.17`. This task did not sign, publish or install those
packages. The historical 807-role journal matrix was not rerun or recounted;
this repair changes no journal or writer-admission contract.

These are source, artifact-reader and Controller activation checks, not real
native-generation, production-login or cross-request lease-recovery acceptance.

## Production observations

The pre-dispatch/after window was **19:42:52–19:43:57 +08:00** (65 seconds,
not an outage measurement). Controller PID changed from `2645039` to `2793404`
and its executable matched the accepted artifact. All **12** worker PID/start
pairs present in that window were retained, along with Machine PID `1232222`
and all three Victoria process identities. Machine stayed online on
`worker-48ad34f5c4615668b75f`, with unchanged workspace revision/hash.

Web/Machine profiles and receipts, host closure/unit hashes and cold roots were
unchanged. System and user failed-unit sets were unchanged. Local and public
HTTPS checks passed `/healthz`, `/version`, exact index/admin/service-worker and
both entry-asset bytes, including cache headers. SPA version remained
`9dcf7c01602e4bf519e697e619761b03`; no Web or Machine activation occurred.

Private evidence is retained at `/tmp/cowboy-zed-operation-hpu5oKZX`: failed
regressions, metadata assertion failures and repairs, targeted/full gates,
immutable build, exact-reader and publication checks, process snapshots and
activation/HTTP audit. SHA-256 identities:

- Final complete gate:
  `bd2e381902ab4807bd473794f8b262e04ff47d148f03c18a3b97cdf1d7e56f8e`.
- Immutable build:
  `332430cf1665707537525c4f39e083884075ef167adf97cbe2132a1c444e073d`.
- Actual-reader audit:
  `269e7a303da394ed7e8c0e49637f9f3adf914ec19309b0b8e171a5c1490846b8`.
- Activation/HTTP audit:
  `dab850836d40a101b3f213d39541cef40222981462ff515b91f05f3055bbaa58`.

The [completion ledger](../plugin-refactor-completion.md) retains the remaining
resource/state, independent post-effect recovery and supported-client exits.
