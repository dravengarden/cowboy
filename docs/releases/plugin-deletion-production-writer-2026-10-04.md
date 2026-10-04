# Production Session terminal-deletion writer — October 4

Hawk now runs the dedicated schema-1 deletion writer. The native build binds
its clean Git revision and requires the fixed root component profile, selected
native executable and existing root reader floor for the configured Machine
and namespace before opening the journal. Ordinary/bootstrap builds stay
read-only, including when runtime variables imitate the writer build flags.
The installed component owner independently guards activation, fallback and
same-generation recovery. Reader-only fallback remains eligible.

This is finite terminal-ID persistence and startup admission. It does not
provide fresh same-ID Session incarnation, continuous worktree ownership,
general state leases, portable writer authority or fencing against independent
sudo/same-user actions. Historical volatile deletes are not backfilled.

## Exact executable acceptance

Implementation source is `6be609469805b51b18c0ae9979e23657b0d9dac4`; the
private namespace harness starts at `6183a69f`. Fresh main integration produced
the deployed `1324bde559cd9587086b9cb949224422003b045d`, published to remote main.
The owning gates passed: Rust formatting, both all-feature and standalone
Machine Clippy configurations, 1,855 all-feature library tests (53 ignored),
and 534 standalone Machine tests (15 ignored). The standalone/full feature
gates clear `COWBOY_PROVIDER_PACKAGE_PATH` as the repository prescribes;
inheriting it in the first manual run caused 12 unrelated Provider failures.
A read-only native Codex review with memory disabled found no actionable
defects; the owning task independently executed the build/test gates.

Two clean, independently built production writer releases have different
embedded revisions and native ELF hashes. They retain the same accepted
`worker-748825b42b4302fe26ca` bundle from
`b97c2724bea23834944ded8af98e2de6729f4256`. The supplied already-active schema-1
reader is a separate read-only fallback. A new ordinary build proves that
runtime environment variables cannot enable its writer.

The 32 groups run actual release launchers/native binaries over real Unix IPC
in private root mount, PID and network namespaces. Synthetic root selections
and floors are bound only inside those namespaces; no live deletion record is
seeded. Coverage includes positive ACK and inode-preserving dedup, ACK followed
by SIGKILL and old/new/old cold fences, competing writer refusal, reader-only
fallback, replaced locks, storage failure without surviving-worker effects,
poisoned further admission, malformed/foreign/nonregular/mutable authority,
foreign profile ownership, untrusted selection links, native mismatch and
bounded closed source parsing. Every child is killed/exited and reaped.

The four staging/file-sync/rename/directory-sync crash checkpoints remain
test-only library fixtures. The production binaries contain no checkpoint
hooks. This finite executable pair is not a historical all-writer matrix,
power-loss simulation, native-generation replacement or physical-device proof.

```bash
nix develop -c just session-deletion-production-conformance \
  OLD_WRITER NEW_WRITER ACCEPTED_READER DEFAULT_READER RECEIPT
```

## Hawk activation

The deployed immutable result is:

```text
/nix/store/imycj67vn65cs581xv4mhhpm9syr8hzs-cowboy-machine-writer-host-release
```

An earlier accepted candidate was refused before dispatch when fresh main
advanced. The task integrated that revision, rebuilt, passed the exact 32-group
matrix again, and activated through the installed immutable component owner.
Transaction `1791122234810898569-1324bde559cd` began at
`2026-10-04T13:57:14.810898569Z` and committed at
`2026-10-04T13:57:26.022247943Z`. The root receipt records succeeded/committed,
published, maintenance, unrecovered and the exact retained worker generation.
Startup logged zero loaded terminal IDs and `writer_enabled=true` at
`2026-10-04T13:57:14.911984Z`.

The bounded activation snapshots retain all 12 worker and six keeper unit IDs,
PIDs and states. Only the resident Machine changed PID, from `3737963` to
`403650`; Controller PID `312802` is unchanged in that window. An independent
Controller rollout changed its earlier PID before this window; it is not
credited as continuity across the whole task. The SPA link, root floor digest
and `.lock`-only live namespace are unchanged. Public health/version/SPA/SW and
Machine deployment health return 200. Installed owner and host closure are
unchanged. Passwordless sudo works and the sudoers digest is identical.

Future resident fixes on this admitted target use
`cowboy-machine-writer-host-release` to retain the writer and accepted pool;
ordinary reader-only artifacts remain explicit compatible recovery targets.
Other Machines and portable installation are not writer-adopted by this release.

Exact ELF digests, observations, component receipt and before/after snapshots
are in [the acceptance receipt](../experiments/plugin-deletion-production-writer-2026-10-04.json).
