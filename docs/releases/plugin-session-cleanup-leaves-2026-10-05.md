# Observed Cargo cleanup leaf identities — Hawk, October 5

Linux cleanup now pins each nondirectory entry with restricted `O_PATH` and
no-follow flags, then revalidates its parent and current-name device/inode/type
before unlink. Observed file/link replacement, disappearance or file mount
crossing retires cleanup before that unlink. Identity acquisition reads no
contents, opens no device and does not wait for FIFO peers. The
[contract](../plugin-session-cleanup-targets.md) links the Linux flag semantics
and records the remaining non-atomic comparison/unlink boundary.

Implementation `0d4bd97586a8a6a2f07fe24bb1e30c1f6d50c1af` is published and active
through integrated source `b56db9833b2dfb14d67b7f91874dffb077b9767a`. The immutable
artifact is
`/nix/store/wsyf1xxpgm0b3gl9jh6pz2fxj33xpz4g-cowboy-machine-writer-host-release`.
Root transaction `1791169108506211110-b56db9833b2d` started at
`2026-10-05T02:58:28.50621111Z` and committed at
`2026-10-05T02:58:35.980275723Z`. Its receipt reports succeeded, committed,
published, maintenance and no recovery. Startup at `02:58:28.588821Z` confirms
the writer remains enabled with zero recorded deletions.

Integrated runtime Rust `893d43bc` passed 1876 all-features library tests and 550
standalone Machine tests; 57 and 17 environment-dependent tests remain ignored.
Both Clippy gates and Rustfmt passed. Native read-only review found no actionable
regressions. Later integration changed only Web and acceptance documentation;
runtime Rust, wire, SDK, dependencies and worker pin stayed unchanged.

Two new filesystem fixtures cover regular and linked original nodes replaced
with new files, links, directories or missing paths after the identity handle
opens, and FIFO contents with no writer. Refusal preserves original and
replacement nodes. The existing ancestor-rename fixture now preserves the
original artifact too, because its parent is checked again before leaf unlink.
Both ignored mount fixtures were separately executed in a private user/mount
namespace. Extended cases bind a same-device foreign file over a leaf before
and after the identity open; original held contents and foreign contents remain
unchanged. Existing directory bind and scan bind cases also pass.

The exact final writer, previous active writer, accepted reader-only fallback
and final default reader passed 32 production conformance groups in private root
mount/PID/network namespaces. These verify journal/startup/IPC compatibility;
leaf-specific evidence comes from the real-filesystem and actual mount fixtures.
No synthetic records entered the live journal and no native production
checkpoint hooks were introduced.

During `02:58:17.147Z`–`02:59:33.300Z`, all 13 workers and six execution keepers
retained identical IDs, PIDs and active states. Machine PID changed from
`3232214` to `3625760`; Controller PID `2648009` remained unchanged. Accepted
generation `worker-748825b42b4302fe26ca`, reader floor, journal entries,
Controller/Web receipts, resolved SPA, host source, installed component owner
and sudoers digest remained unchanged. `sudo -n true` succeeded. Health, version,
SPA, service worker and Machine deployment-health endpoints returned HTTP 200.

The [machine-readable evidence](../experiments/plugin-session-cleanup-leaves-2026-10-05.json)
contains exact artifacts, native hashes, conformance observations, mount-fixture
command and before/after receipts. The final identity comparison and name unlink
are still not atomic; a later replacement nondirectory can be unlinked. Same-node
content mutation is not a new object identity. There is no filesystem freeze or
atomic snapshot and partial effects are not rolled back. General I/O deadlines,
continuous ownership, durable Session incarnation and portable writer admission
remain open. Non-Linux Unix retains its previous pathname fallback.
