# Session deletion namespace path identity

Preparing production writer admission exposed a reader/writer ownership bug:
`Journal::open` canonicalized the namespace before opening it. This followed a
final namespace link despite the subsequent `O_NOFOLLOW`, and retained the
resolved target instead of the caller's logical namespace path. Replacing a
parent alias could leave admission attached to an obsolete target.

The journal now makes the caller's path absolute without resolving symlinks and
opens that namespace with `O_DIRECTORY | O_NOFOLLOW`. Initial final-component
links refuse before lock creation, record loading or broker binding. Retained
namespace checks still compare the logical path's directory device/inode and
lock identity to held descriptors. Parent aliases remain valid initially;
observed replacement ends reader/writer admission before a later journal write.
This is the existing observed-identity boundary, not a race-free same-user or
administrator mutation fence. The writer remains hard-coded off in production.

New unit fixtures cover both reader and private writer final-link rejection and
parent-alias replacement, retaining external bytes and forbidding new records.
Disposable broker processes exercise both reader/writer startup refusals before
Welcome/socket creation. The opt-in `session_deletion_releases` test runs exact
independently supplied old/new immutable Machine binaries with linked empty and
committed namespaces. The old reader must admit and create a target lock; the new
reader must refuse without creating a target lock or changing record/link bytes.
The complete existing 31-process upgrade/rollback/refusal matrix must also pass.
Source gates, native identities and activation receipts follow below.


The complete source gates pass 1,848 all-feature tests (53 ignores), 529
Machine-host tests (15 ignores), formatting, all-feature Clippy and the packaged
standalone-host Clippy targets. The incoming main integration changes Web,
documentation and developer-shell OpenSSL inputs, with no Rust or native boundary
changes. The clean published integrated source is `0d41edebd42ce891998e577c7605e0e4190a53e6`.

The exact immutable artifact `/nix/store/7sqzl815jzdrq4x8mm9r3dm1c0n30ar8-cowboy-machine-release` retains all six independently
accepted companion paths/digests, worker source
`b97c2724bea23834944ded8af98e2de6729f4256` and generation
`worker-748825b42b4302fe26ca`. Actual predecessor
`/nix/store/8gygd38dk4qh1ln771idi1ph2nhh875x-cowboy-machine-release`
admits both empty and committed linked namespaces, creating the target lock.
The new package refuses both before broker binding and lock creation; committed
record and link bytes remain unchanged. The existing 31-process reader-pair
matrix also passes: 11 SIGKILL/reaped admissions and 20 startup refusals. Combined
acceptance uses 35 actual Machine processes, with 13 SIGKILL/reaped admissions
and 22 startup refusals. Writer process tests use the compiled private test
fixture, not an independently accepted production writer executable.

Root owner transaction `1791114101823885062-0d41edebd42c` started 2026-10-04T11:41:41.823885062Z and committed
2026-10-04T11:41:50.903761266Z, with succeeded/committed, published true, recovered false
and activator success. Resident Machine PID changed 2790245 to
3031824; all 13 worker and 5 keeper IDs/PIDs/states stay identical.
Controller PID 2336663 and receipt, Web profile and root reader
floor stay identical; all five HTTP checks return 200. Startup reports zero
deleted Sessions and writer false. The namespace contains only `.lock`, the
floor SHA-256 remains `26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`, and sudo remains noninteractive with
unchanged sudoers/installed-owner digests. See
the full receipt (in Git history).

Production writer admission still requires its own compatible activation/fallback/
recovery contract and actual immutable writer crash/reopen/failure acceptance.
The installed owner still rejects nonzero writer metadata. This prerequisite
fix changes neither that policy nor the selected trusted-administrator boundary.
