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
