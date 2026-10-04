# Staged Machine host integrity — October 4

Component reconciliation verified the freshly downloaded artifact digest and
publisher signature, but reused an existing cached executable without comparing
its bytes. An altered cached host could therefore run its health probe and be
published under the authenticated artifact's manifest. A correctly signed probe
could also modify the staged payload before publication.

Machine host reconciliation now prepares expectations from the authenticated
download, checks staging before writing its manifest or running its probe, and
checks it again before changing active, rollback or command pointers. Raw hosts
must be regular files with the exact signed bytes. Archive hosts must have the
exact regular-file/directory tree and file digests, including companions and
empty directories. Expected archive paths are normalized before extraction;
duplicate files, file/directory conflicts and special entries refuse. Staged
links and extra entries refuse. File reads use no-follow/nonblocking opens and
fixed-size hashing buffers. Other component kinds keep their existing behavior.

An altered cache is refused without repair or probe execution. A signed probe's
effects remain on refusal; no rollback of arbitrary probe effects is claimed.
The two checks observe staging at separate points; they do not create an
immutable filesystem snapshot or fence a concurrent administrator. This does not authenticate an
already cached host at a later launcher start, supply signed bootstrap recovery,
create a portable persistent floor or admit committed portable deletion state.
The production deletion writer remains disabled, and the user's retained sudo
authority remains unchanged.

Source acceptance covers changed raw bytes, a raw symlink, changed archive
companions, extra archive files and archive symlinks. These candidates refuse
without executing a marker probe, rewriting a retained manifest or changing
retained pointers. Raw and archive probes that alter their own staging run once
but cannot publish. Unchanged raw/archive hosts activate both on initial staging
and cache reuse. Additional archive fixtures reject duplicate and normalized
duplicate files, both file/directory conflict orders and FIFO entries. All keys,
payloads, markers and state in those tests are synthetic temporary fixtures.

Implementation commit `5afd057e` was integrated with fresh main as
`4f70a607f6033f47afd0c48bb57f109454f92baa`. The incoming Sessions-folder
changes are Web-only; their lint and type checks passed after integration, and
Web tests/build also run against the merged source. This task activates only
Machine. Final activation evidence follows after the immutable release. No
production portable install or signed component record is published by this slice.

The first complete gate refused the previously integrated native app-shell
change `eebf0a5c`: its source digest had not been recorded in the component
registry. Append-only release 3.38.0 records app-shell 1.1.19 and its new digest;
there are no dependent component/Plugin changes. Historical registry entries
and all Plugin pins remain unchanged. This repairs the deterministic gate rather
than weakening it. This task does not activate Web or native app releases.

The complete gate's static checks, Clippy with warnings denied, dependency audit,
type/feature checks and all-feature suite passed (1,784 passed, 42 ignored).
Its parallel independent Machine suite hit two existing failures:
`committed_terminal_ids_survive_close_and_read_only_reopen` reported a busy
journal after drop, and `empty_success_alone_cannot_admit_a_bootstrap` did not
reach its expected refusal assertion. Neither was suppressed. The entire
Machine suite was rerun serially: 476 passed, five ignored; both failures did
not recur. This is consistent with transient concurrent subprocess interaction,
not proof of its cause or a fix to those tests. Code-adapter and Zed suites, merged Web tests, isolated PostgreSQL fixtures
and release builds all passed in the resumed gate. The gate was completed in
these stages; the initially failing `just check-compact` invocation itself is
not reported as a successful run. No production guards or tests were relaxed.
