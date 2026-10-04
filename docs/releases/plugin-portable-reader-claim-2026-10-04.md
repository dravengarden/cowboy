# Signed portable Session deletion reader claim — October 4

Portable component signatures previously had no typed declaration for the
Session deletion reader. Nix release-source metadata declared the reader, but
that was a separate contract. A future portable floor must bind an independently
verified publisher claim to the exact selected artifact, not infer it from
current binary behavior or an unsigned extra manifest field.

Cowboy source `17aa7ca69ab3556e993480cc9adfc6b6b71c8326` adds optional
`session_deletion_journal` to `DesiredComponent`. The closed object requires
`reader_schema: 1` and `writer_schema: 0`, and is permitted only on the singleton
`machine_host` with an empty slot. Controller manifest acceptance and Machine
preflight refuse unsupported readers, enabled writers and wrong owners. Invalid
Controller reloads retain the previous accepted set. The production deletion
constructor still hard-codes writing off.

Absent/null claims preserve every byte of the existing `cowboy-component-v3`
length-prefixed transcript. Present claims select `cowboy-component-v4` and
append the compact canonical 37-byte JSON declaration. There is no old-domain
signature fallback. Adding, changing or stripping a claim cannot reuse the
signature; an old receiver discarding the optional field cannot verify v4.
The [publication contract](../machine-operations.md#component-publication)
specifies the exact domain, field order and bytes.

This is a signing prerequisite, not portable reader/recovery admission. Even a
valid signed declared reader still refuses committed portable state before fetch,
staging or installation-pointer publication. No persistent portable floor,
verified bootstrap recovery, writer admission or independently privileged old-tool
revocation was added. No new signed production component record was published.
The user's [retained administrator scope](../plugin-activation-authority.md)
remains unchanged.

Pinned-shell `just check-compact` passed, including Clippy with warnings denied,
all-feature unit tests (1,780 passed, 42 ignored), Machine-host library tests
(472 passed, five ignored), enabled integration targets, Web checks, PostgreSQL
fixtures and release builds. Four new tests cover exact old/new signing vectors,
real synthetic-key signature verification, mutation/stripping refusal, typed
Controller acceptance/reload retention, pre-fetch Machine refusal and continued
committed-state rejection with unchanged synthetic evidence and pointers.
Only independently generated temporary fixture keys were used.

Implementation commit `1684816b` was merged with the concurrent native Swift
build-adapter change `2158be02`. That merge changes native tooling/docs, not the
tested Rust reader implementation; its owning `native-shell-check` passed after
integration. This task builds and activates Linux components only and makes no
iOS build, device or IPA publication claim.

The initial clean candidate built Controller
`/nix/store/hp84w15wlmv4mc7nr9fqzih43jhhcd3p-cowboy-controller-release` and Machine
`/nix/store/x62klvdrsag93amylcpl42svxm54y8r5-cowboy-machine-release`.
Its first Controller dispatch refused during ancestry preflight because remote
main advanced to the concurrent native repair merge `8c10ba19` while those
artifacts were building. No detached transaction was dispatched, no new receipt
was written and no service was restarted by that refusal. The owner gate was
retained; the task integrates fresh main and rebuilds immutable metadata before
retrying. Final successful activation evidence, when obtained, is recorded below.

Final source `406471a28de430debf6f8363b44abc3e621589d7` integrates that fresh
main. The owning native-shell checks passed again; the Rust source and executable
digests remain identical to the complete-gated initial candidate. Only immutable
release metadata needed rebuilding. The final clean narrow artifacts are:

- Controller: `/nix/store/8v8scdvvpslh2ivw3rbdl6j2inkfcb9p-cowboy-controller-release`.
- Machine: `/nix/store/jxmq93y2ifqxvgslpk22j7i7dd1v2qxy-cowboy-machine-release`.

The installed owner dispatched both components through their separate recipes.
Controller transaction `1791079277476212104-406471a28de4` succeeded, published,
at `2026-10-04T02:01:33.525825925Z`. Its receipt has `maintenance: false`.
Machine transaction `1791079381270657298-406471a28de4` succeeded, published,
at `2026-10-04T02:03:09.809841529Z`, with `maintenance: true`.
Neither component retained an incomplete journal; no new failed unit was observed.
No Web, host configuration or installed Plugin activation was performed.

Samples at `2026-10-04T02:01:04.697Z`, `2026-10-04T02:02:11.157Z` and
`2026-10-04T02:04:32.600Z` retain all 13 ACP worker and three keeper PIDs.
Controller changed only in its own stage, from PID `486493` to `959309`;
Machine retained PID `1928418` during that stage, then changed to `965129`
during its separate maintenance. The running native paths/digests match the
final artifacts. No worker pool generation changed: the release and public
inventory both report `worker-6ede7a91cc8b8b3402d4`.

HTTPS health/version/SPA/SW/deployment-health returned 200 after each stage;
HTML/SW retained `no-store`. The resolved Web target and SPA version
`798bda6db1a3a8958a6102125058e8e2` stayed unchanged, as did reader-floor and
sudoers SHA-256 values. Deletion state remained only `.lock`. The new Machine
logged zero deleted IDs with `writer_enabled=false` at
`2026-10-04T02:03:01.355079Z`. These are bounded Linux process observations,
not native-generation resume, portable installation or supported-device acceptance.

The [machine-readable evidence](../experiments/plugin-portable-reader-claim-2026-10-04.json)
contains both artifact identities and native digests, the unchanged installed
owner, initial preflight refusal, two success receipts and three process/HTTP
samples. Portable persistent floor, bootstrap/recovery reader verification,
committed-state admission and production deletion writing remain closed.
