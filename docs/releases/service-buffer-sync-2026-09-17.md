# Service-owned synchronization — 2026-09-17

The Service continuation is published on `main` and active on Hawk. This is a
**Controller-only release**, not production end-to-end synchronization: the
resident Machine, installed Zed, ordinary Review and browser synchronization
consumer have not been upgraded or connected by this slice.

## Exact artifacts and release

Controller source is clean commit
`c48b3d12c836dda1a6e541f7882e5c15b352a515`, including the final job-exclusion
repair. Machine candidate source remains
`ef84a341e4ca97144be7353ce5fef5f5db2fabc8`; later changes affect Service code,
test harnesses and documentation, not its implementation. The private adapter
`1.9.0` and server `1.0.0` are the unchanged portable inputs from the
[Machine candidate](machine-buffer-sync-candidate-2026-09-17.md).

| Artifact | Immutable release / SHA-256 |
| --- | --- |
| Controller | `/nix/store/q61apwxgfz8w30a280xk1sx8x7cg785c-cowboy-controller-release` |
| Controller ELF | `292e296c2aed31a42b7cb85b6abcb85eb9852435a563c8dacbeede4b99cefc78` |
| Machine candidate, not activated | `/nix/store/d5s9s8l6m3wph6lmsblzd41rhqy3kysa-cowboy-machine-release` |
| Machine wrapped ELF | `52a7f05af88c63db4895f48a15548b6f70665fb4a4a5c4426e588c0b54faba49` |
| Private adapter ELF | `b9b5c5da9bf47e54a31bf807f6ac21387a3e9ed6a895da89b551e80016bdc5f3` |
| Private server ELF | `da41ec6baee1cbf714b809fcd912dd200be150c634cbe4a6ec6939e323da8131` |

The machine-owned Controller transaction is
`1789608195215990063-c48b3d12c836`: `succeeded`, `committed`, `published: true`,
`maintenance: false`. It started at `2026-09-17T01:23:15.215990063Z`; the
activation unit completed successfully at `09:23:26 +08:00`. Its exact
predecessor/recovery release is
`/nix/store/sd524wjyvjlfllzr70y40rbf489r0sv0-cowboy-controller-release`
(`255fe2b7`). No Machine/Web activation, production Plugin signing/installation,
policy/credential change, SQL migration or operation replay was issued.

## Accepted behavior

The [Service contract](../plugin-service-buffer-sync.md) requires original
resource ownership, a current product Operator and separate preparation/fresh
confirmation. It binds Session incarnation, Machine connection, purpose and
exact content. IDs and serialized declarations are not grants. Protocol 19
cannot use the new path; generic adapter forwarding remains closed.

Apply commits uncertainty before I/O and never replays. Its task survives the
HTTP observer, retaining exclusion across unknown outcomes. Query and retirement
use only original evidence, with fresh permission. Inert expiry cannot dispose
of attempted effects. Ordinary Machine reads/releases now expire forgotten
inert reservations too; this Machine fix is accepted in the candidate, not yet
active on the resident Machine.

Final review found a Service job-lifetime race: recording a result cleared
`busy` before the final asynchronous permission check, so an older job's Drop
could erase a successor's exclusion. Admission now remains owned until that
job drops. A dedicated regression exercises Apply/Query/Retire during this
response-authorization interval. Terminal observations do not grant parallel
mutation or retirement while the prior job still owns admission.

## Verification

The complete pinned `CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 just check-compact`
gate passes: **1,409 main Rust, 333 standalone Machine, 26 core-adapter,
75 private-adapter, 1,613 Web and 18 isolated PostgreSQL tests**, plus formatting,
strict Clippy, dependency/type/feature checks, composition and release builds.
Main/Machine/private-adapter retain 32/two/two explicit ignored tests; applicable
process tests ran separately. Final-source formatting and strict Clippy were
repeated successfully. All eight executed `nix flake check` checks pass.

The exact Controller/Machine/native pair above passed the
[v3 connected Code gate](../plugin-code-connected-conformance.md) **twice in
fresh isolated fixtures**, all 11 checks each. Each receipt records three
connections, 96 real replies, six held replies, one discarded reply and one
connection cut; there is exactly one synchronization Apply, Query and Retire.
Both receipts have `accepted: true`, `cleanup: true`, and no failure.

This includes real fixture login and signed installation, shared-owner refusal,
Prepare before HTTP uninstall and Apply afterward on the retained native
process. The relay discards an actual Apply reply and waits the unmodified
40-second timeout. Duplicate Apply only observes Unknown; original-ID Query
establishes Applied and exact content reads agree. Cancelled retirement drains
without resend. Connection replacement and Controller restart cannot adopt or
claim retirement of the old operation. The separate installed-runtime and
private native conditional-sync gates also pass against the same static pair.

Early complete-gate failure was an obsolete protocol-19 proxy assertion;
protocol-20 positive/negative cases are corrected. Early connected fixtures
correctly received native `refused/changed`, not Applied, when source metadata
changed around preparation. The stable-source success fixture now pre-seeds the
file and changes real Unicode bytes while preserving size/mtime, isolating
explicit content synchronization from native metadata/reload events. It still
requires changed native text, exact hashes and every conditional check. No
runtime guard, timeout or success condition was weakened to accept those runs.
Earlier failed receipts remain retained separately. Existing unrelated build
and dependency warnings were not suppressed.

The canonical `release-cowboy-plugin` skill's Controller preflight was applied:
candidate, actual next-transaction recovery and cold Controller each read the
same **84-release Catalog twice**, and their actual-Service configuration-only
host/telemetry reports match. This is not live authentication, a new telemetry
writer grant or cross-end export acceptance. Private process environment and
destination policy were not recorded.

## Production continuity and limits

The audited window is **09:22:47–09:23:49 +08:00**, not an outage duration.
All **15 original workers and four private Code processes** retain their
PID/start identities. Resident Machine PID `1974549`, worker generation
`worker-4208d4d141de95cf9feb`, Machine/Web profiles and receipts, installed Plugin
identities and saved Zed operation, Victoria processes, host/cold closure and
failed-unit sets are unchanged. Workspace manifest and advertised revision
`c3ad3da0e46517dccef201a4dee00d03a32a9743` are unchanged as well.

The new Controller PID is `3510402`. Local/public health and version pass, as do
exact bytes/cache headers for the unchanged SPA, admin, service worker and two
entry scripts. Web version remains `4d8405ee6e987d3fa911a2bb540bdf0b`. The new
Controller epoch reports zero dropped intents and failed persistence batches,
healthy persistence and a connected Machine. These are HTTP/process checks,
not browser/device or production synchronization acceptance.

The earlier Web `recovery-required` journal is preserved unchanged. The two
historically rejected persistence intents remain unknown and were not replayed.
This release does not resolve either independent incident.

Still required: typed browser synchronization ownership and explicit
confirmation/unknown presentation, ordinary Review content/position lifetimes,
owned navigation destinations, separate authorized Machine maintenance, exact
signed Zed rollout, supported-device acceptance and independently authorized
post-effect/restart recovery. Process-local evidence cannot be reconstructed
into a replay grant. Other [completion exits](../plugin-refactor-completion.md)
remain open; the Plugin refactor is not complete.

## Retained evidence

Private evidence root: `/tmp/cowboy-service-buffer-sync-mcNNOvVL`. Initial
`connected-receipt-*`/preflight artifacts are superseded only by the explicitly
named final records, not overwritten.

| Final record | SHA-256 |
| --- | --- |
| `check-compact-2.log` | `2ea4421a4a4c575b420944e356809a0ce8e20836459eae05727df1cb182f3620` |
| `nix-check-final.log` | `c9d7a51ad8f7cf3a6b388d420d7f890c27a90481762ad9e8efe0aafa027fc64d` |
| `verification-final.log` | `29be7e4035ca695cd4509eeba3caf28b056b4659b03054757276b4a873f360d1` |
| `connected-final-1.json` | `73186e02482e938800cf48431006dca6bb89ede545430614d05e9f4d3054094c` |
| `connected-final-2.json` | `fe8ab29b9bc1709cc5b2dd890bc3048531790bca771635bc25dc8d492a8f755a` |
| `activation-audit.json` | `a481ba360216539a2185dd796f5212c2cec743d3dae0e6a149c84bbc7e814955` |
| `final-http/receipt.json` | `517f681a18bcf9542bc39b49a68302c9538c662b42dc583693050af29bddff2c` |
