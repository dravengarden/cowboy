# Private synchronization ownership and Hawk upgrade — 2026-09-16

**Zed 1.8.0 is signed, published and installed on Hawk.** Both existing private
Code processes and all 16 observed workers retained their PID/start identities.
No Machine maintenance was performed. This accepts private adapter ownership
exclusion, not core synchronization authority, Review cutover or restoration.
The later persistence admission bug was separately
[repaired on Controller](persistence-admission-2026-09-16.md). Recovery of its two
historical rejected intents remains unknown; installation acceptance must not be
reported as proof of their restoration.

## Exact source and release

Runtime source is published commit `00df87656e058a8da21294d19f7d730d3a458021`,
rebased onto `73aa8541`. Later Web/documentation commits do not change these
Plugin or Machine candidate bytes. Only Linux x86_64 is declared.

| Identity | Accepted value |
| --- | --- |
| Plugin / private adapter | `zed` / `cowboy-zed-adapter`, `1.7.0` → `1.8.0` |
| Private server | Unchanged `cowboy-zed-server 1.0.0` |
| Upstream source | Unchanged `aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45` with existing source-owned overlays |
| Publisher | `cowboy-first-party`; configured Ed25519 signer and independently selected public verifier |
| Composite artifact | `sha256:56474a7197fb8ba30d401236e780a35f107a9e9e7a5ab9869445c0d53a425d20` |
| Package | `sha256:ec2946355a627e16020208a051f2d2ac2130ae7f0f49928d7d6ceb76f9b74182` |
| Adapter ELF | `sha256:a69c9a120f8ebb62f1f2e0e3b74452eef26a5f17247186e372d1362ac57e167e` |
| Server ELF | `sha256:da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |
| Contract fingerprint | `sha256:b4d6235ab5ae0a908ffe0cc55e4de9fdac96ea418b92604eb989579d610fbc19` |

The prior digest/fingerprint are recorded in the [1.7.0 rollout](zed-native-sync-2026-09-16.md).
Both runtime components remain static, with no ELF interpreter or shared-library
dependency. No upstream dependency, ordinary Zed installation, Provider account
or credential was changed. Authentication contracts do not apply to this Code
Plugin. The outer release remains schema 1 with Code payload 2 and SDK 1.8.

Immutable adapter: `/nix/store/69map96pgqhrnw8b3wiz8pxkdqrs4mq4-cowboy-zed-adapter-x86_64-unknown-linux-musl-1.8.0`.
Unchanged server: `/nix/store/lywsfx0zmq03ml9rjlqhhqw9dmp0gdvr-cowboy-zed-server-x86_64-unknown-linux-musl-1.0.0`.

## Accepted behavior and gates

The [ownership contract](../plugin-buffer-sync-owners.md) resolves only the
original open owner, counts all owners across aliased native buffer IDs and
refuses unresolved opens. Preparation reserves the buffer; Pending/Unknown
retain exclusion without expiry. New opens/navigation cannot introduce an alias.
Existing unrelated reads/releases remain available. Apply admits once; original-ID
queries cannot replay it or turn a missing native outcome into restoration.
Successful synchronization invalidates old coordinates before releasing the
fence, including when its reply precedes native text events. Terminal cleanup
can resume after observer cancellation without resending the effect.

Twelve new private tests and one Machine gate test cover those boundaries.
The real private-server gate exercises two owners, shared-owner refusal, actual
text synchronization, retained-owner content reads, duplicate Apply with no new
native request and cleanup after path removal. Temporary signed install,
uninstall drain and retained-generation reactivation also pass.

Both eight-group connected Code runs pass with exact final runtime bytes:
actual Controller `n2r5956h…` and either the still-active Machine `yrdndji8…` or
the final candidate `avhgy5q9…`. These use disposable password login, enrollment
and installation, not production credentials. They accept the existing Code
ownership/read/release chain, not a public synchronization continuation.

The complete `CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 just check-compact` gate
passes: **1,358 main Rust, 314 standalone Machine, 26 core-adapter, 74 private
adapter, 1,608 Web and 17 isolated PostgreSQL tests**, plus formatting, strict
Clippy, dependency checks, type/feature checks, composition conformance and
release builds. Main/Machine/private-adapter retain 31/two/two explicit ignored
tests; applicable actual-process gates ran separately. `just plugin-check` and
the full `nix flake check` also pass. Existing dependency/lint/chunk warnings were
not suppressed.

After integrating the subsequent independent Web commit `0b256bc0`, native-shell
and website checks, TypeScript checking and all **1,610 Web tests** pass again.
All 152 local links in the affected documentation/index resolve. No core or
Plugin runtime file differs between that commit and the accepted release source.

Two earlier full attempts are retained as failures: a self-executing probe could
not spawn while builds overlapped, and a parallel Operator lock-release test
reported a still-owned lock. Each failed test passed three isolated repetitions;
the final complete gate uses the preceding release's single-test-thread setting.
This is not a claim that the parallel-test interactions were repaired.

## Publication and installation

The canonical release skill required an independently verified staging Catalog.
Actual active/next-transaction recovery Controller `n2r5956h…`, retained
historical predecessor `fc86l6vr…` and cold Controller `cc09k6l7…` each read the
complete **84-release Catalog twice**, both before and after publication. Their
actual-Service configuration-only host/telemetry preflights pass. No policy or
environment was copied into the evidence.

The complete build gate regenerates an unsigned package envelope. The first
publication precheck refused that unbound output before any live write; final
URL/runtime binding, signing and independent verification reproduced the exact
previously accepted staged bytes. Publication then completed at
`2026-09-16T13:44:25.265Z`. All three package/runtime HTTPS downloads match their
signed hashes. The primary Catalog reports the exact `1.8.0` digest/fingerprint,
`ready`, `code_intelligence` and Linux x86_64. The official website's
`plugins.json` also advertises `1.8.0` after the main-branch push.

The existing host Operator submitted exactly one normal durable upgrade:

- Operation: `hawk-zed-1-8-0-sync-owners-56474a7197fb`.
- HTTP 204; Service `completed`, Machine receipt `applied`.
- Installation: `installation-f01bdbb398160911f2ecd80da2c9749ad548ad9152eef920adef49b805411cf5`.
- Inventory: exact `1.8.0` identity, active, no reconciliation fence; `1.7.0`
  retained as the rollback generation.

The observation window is **21:44:43–21:45:13 +08:00**, not an outage duration.
All **16 worker** and **two existing private Code process** PID/start pairs
survived. Existing Code owners remain on their old runtime; this does not claim
that they migrated to 1.8.0. Controller, Machine, Victoria, host/cold closure,
unit hashes, failed-unit sets, workspace identity and other installed Plugins
were unchanged. No installation-pointer edit or forced process retirement was
used.

A separate Web deployment crossed that window. The first strict audit correctly
refused an unchanged-Web claim. The accepted audit instead binds the exact
independent transaction `1789566304429688338-0b256bc06bae`, committed source
`0b256bc06baec24c02be423786f769b2233920a9`, its predecessor and immutable Web
artifact. This descendant adds desktop composer file dropping; it does not alter
this release's core/Plugin bytes. This task did not activate it. Local/public
health, version and five exact SPA/admin/SW/entry files and cache headers pass
against that actual Web root; its version is `4d8405ee6e987d3fa911a2bb540bdf0b`.

## Later persistence degradation — unresolved

Update: the [15:38 UTC Controller repair](persistence-admission-2026-09-16.md)
restores current health; historical event recovery remains unknown. The following
is the retained pre-repair observation, not a claim that health is still 503.

At `13:45:36.204Z`, after the accepted installation observation window, the
unchanged Controller logged two rejected append events (estimated 437 and 132
bytes) while its pending queue held approximately 15.6 MB. Its existing
8 MiB admission rule allows one oversized event into an empty queue, then rejects
additional append events while over budget. The exact still-active Controller
source `2fb32d52` contains this rule and the sticky degradation flag.

Final read-only checks returned **HTTP 503 `persistence degraded`** both locally
and publicly. Metrics report zero queued events/bytes, two dropped intents and
zero failed database batches; Machine is connected with 16 workers. Disk space
and inodes are available. The saved installer still reports `completed/applied`
and the exact installed 1.8.0 generation. No Controller/Machine restart, journal
edit, counter reset or event replay was attempted. The rejected event contents
and their recovery are not established by these counters.

The earlier healthy HTTP receipt remains a bounded historical observation, not
a claim of continuing health. Read-only evidence is retained as
`later-{local,public}-health.txt`, `later-persistence-metrics.txt` and
`later-persistence-events.txt` in the private evidence directory. Persistence
admission received the separate repair linked above; independently justified
historical recovery remains open. Draining the queue or restarting cannot prove
restoration of the lost intents.

## Verified Machine candidate, not activated

`/nix/store/avhgy5q98wk57imclw2d2hydfgmjgsas-cowboy-machine-release` adds an
explicit refusal of private synchronization commands before generic runtime
selection. Its actual-owner configuration-only preflight returns the exact
Machine schema and writer state `unconfigured`; no writer policy was enabled.
Candidate, actual recovery Machine `yrdndji8…` and cold bootstrap `j7lix2f4…`
pass all **72 installation-reader checks**. This proves the supplied reader
floor, not a new production writer grant or full restoration.

That candidate's maintenance activation is **pending separate confirmation**:
it would retire current Code connections. Hawk still runs `yrdndji8…` and
`worker-4208d4d141de95cf9feb`. Its old generic route already requires a worktree,
which the closed private synchronization request refuses; no Controller endpoint
exposes the new commands. Installation did not enable core synchronization.

## Evidence and remaining work

Private evidence root: `/tmp/cowboy-sync-owners-guBAzicu`. Selected SHA-256:

| Receipt | SHA-256 |
| --- | --- |
| Actual Machine connected gate | `1ec2adda2fcf26038d1a3e06c1b90534fe9add7b4c61d977e5cf82a9a6f99a6a` |
| Final candidate connected gate | `d01974cdcd2dca66a469c177c5178772364a7ab37357370a0b8dc5c12f85fad4` |
| 72 Machine reader checks | `2b40e7fe596d334d80a03abe591982b8adb2099e172bf4d2e8f9710e3813945d` |
| Complete serial source gate | `fe289ef5bbb62c87e0eef9c1983d5140f484ce1df3930d9f67ca4fba095236f1` |
| Full flake gate | `ee295bac6237a528692f804688ec9e2da03b2ccbd4a2f4d9afb4116fd1e00600` |
| Production installation audit | `8bff8d0538f31d5cb6c9100a82eb50ac5e1e4db027f8addfbd416642465e8ce2` |

The immutable publication receipt is
`/var/lib/cowboy/plugin-catalog/receipts/zed-1.8.0-56474a7197fb8ba30d401236e780a35f107a9e9e7a5ab9869445c0d53a425d20.json`.

Still required: core purpose/authority and loss handling, exact owner-capability
probe and original-generation continuation, actual Review and owned-navigation
integration, independently authorized post-effect recovery, general live graph
and state leases, managed Victoria cutover and supported account/device checks.
The private purpose enum and native protocol probe cannot provide those grants.
The [completion ledger](../plugin-refactor-completion.md) remains open.
