# Explicit recovery of a damaged floor anchor — October 4

The previous offline command could rebuild a missing anchor package but refused
an existing damaged directory. Linux installers now offer an explicit additional
`--quarantine-damaged-anchor` flag, requiring both `--anchor-manifest` and
`--anchor-artifact` with `--restore-floor-selection`.

All preceding proof boundaries remain: a surviving private floor, empty deletion
dataset, bound publisher, exact original canonical signed proof and artifact,
bounded regular input reads, closed archive grammar, original absolute selection
pointers and regular cache parents. No publisher code executes, no network input
is fetched, and no version, key, floor, bootstrap or identity is replaced.

An intact original generation returns idempotently. A missing generation still
uses atomic no-replace publication. A damaged generation may be exchanged only
if it is a regular directory and the additional flag was supplied. The original
directory's device/inode is captured and checked before exchange. Linked or
non-directory anchor destinations remain refused.

The fully authenticated replacement is built and synced inside an owned private
`component-anchor-quarantine` directory directly below the canonical state.
Existing quarantine directories must belong to the caller, have no group/other
access and be regular directories. Fresh ones use mode 0700. Quarantine sits
outside `components`, so normal cache pruning does not traverse retained damage.
After rechecking floor, publisher, selection and original directory identity,
Linux atomic `RENAME_EXCHANGE` publishes the complete replacement at the original
anchor path and retains the damaged directory at the unique
`anchor-<digest>-<random>.retained` staging path. Both parents are synced; ordinary
anchor authentication and selection recovery then complete. Success receipts
include the absolute `quarantine` path when an exchange occurred.

The old directory's files, modes and links are retained without traversing or
executing them during publication. No automatic cleanup exists here. Failure may
leave an unselected authenticated staging directory; an interrupted exchange may
leave retained damage even if no success receipt was delivered. Repeating against
the repaired anchor authenticates idempotently and leaves retained directories
untouched. If the filesystem cannot exchange these paths (including a separate
cache mount), it refuses without a move/delete fallback. This does not establish
a full power-loss proof or protect against concurrent same-user/admin mutation.

Default recovery still refuses damaged anchors. Non-Linux quarantine publication,
missing-floor reconstruction, publisher rotation, committed deletion admission
and deletion writer activation remain unadmitted. Administrator sudo remains
available. Production recovery is not invoked as part of deployment.

## Verification and production receipt

Source gates, exact immutable installer acceptance and production observations
are appended after completion. This slice uses `cowboy-machine-host-release`
and preserves the independently accepted worker pin
`90ec4edae56349cb06b9f39f13a0086197b5356b` and its six companion paths/digests.


The source gate passed on integrated implementation
`576dc04095affa87cb3d106fb801da1dac6c6ff3`: all-feature and default Clippy,
formatting, 1,820 all-feature tests (51 native/fixture ignores), 504 standalone
Machine-host tests (13 ignores), and the complete Provider gate. The first
interleaved run recompiled the same test executable while it was executing;
two self-launching child fixtures reported ENOENT. The complete final gate then
passed with no overlapping recompilation of that executable.

The immutable installer in
`/nix/store/gb7rkvrj99wqqgm941ycamx9ybd1xpsl-cowboy-machine-release`
passed 58 quarantine cases across raw/archive packages. These cover default
refusal, retained byte/mode/link evidence, partial and damaged manifests,
artifacts and executable payloads, unexpectedly enlarged artifacts, extra archive
files, empty-cache restoration, original dangling pointers, intact idempotence,
repeat quarantine retention, cache-pruning exclusion, closed proof inputs,
committed-state refusal, unsafe destinations and private quarantine requirements.
The preceding immutable installer refuses the new flag without changing the
complete disposable state. Success receipts identify the retained path and
writer false. Current ordinary startup authentication passes after each accepted
recovery, and no publisher execution marker appears.

The same native candidate also passed the preceding 36 absent-anchor cases,
28 pointer cases, 5 signed-refresh cases, 9 signed startup cases, 24 cached startup
cases and bootstrap old-package negative control. Damaged-cache classification
checks file sizes and archive shape against the captured signed replacement
before normal cache authentication, avoiding unbounded reads of enlarged or
unexpected cached payloads during this command. The implementation and tests do
not change ordinary startup authentication bounds.

Production artifact and bounded observations follow after latest-main
integration and metadata rebuild. No live floor-bearing recovery is performed.


Production activated clean published source
`48ba48b9442d533c000d277bdfb71799157c9528` from the final host artifact
`/nix/store/m86n5bf0vlqmcn6gk1jzcmlkqi5n15dm-cowboy-machine-release`.
Its native Machine and installer bytes equal the accepted candidate, and the
entire immutable test matrix passed again against this exact final artifact.
All six companion paths/digests and retained source
`90ec4edae56349cb06b9f39f13a0086197b5356b` remain unchanged from the preceding
production bundle; default generation remains `worker-dc63f38423cbd1971eda`.
The installed owner stayed at SHA-256
`9e1294320f0242363dc426aa856b602c3e55816a250544334e2a353bae545f8f`.

Independent root transaction `1791108507466320305-48ba48b9442d` started at
`2026-10-04T10:08:27.466320305Z` and committed successfully at
`2026-10-04T10:08:36.099027685Z`; published true, maintenance true,
recovered false. The bounded before/after observations at
`10:06:46.176Z` and `10:09:23.618Z` are in
the production receipt (in Git history).
Machine PID changed `1434181` → `1737365`. All 13 worker and 5 keeper unit IDs,
active states and PIDs remained equal; Controller PID `812989`, its component
receipt and Web profile remained equal. `/healthz`, `/version`, `/`, `/sw.js`
and Hawk deployment health all returned 200. Root reader floor SHA-256 stayed
`26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`;
portable deletion state still contains only `.lock`. The new startup journal
records `deleted_sessions=0 writer_enabled=false`. Production quarantine remains
absent; no live repair, writer activation, active-session recycling,
Controller/Web/OS activation or pool change belongs to this transaction.
`/etc/sudoers` SHA-256 remains
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`
and `sudo -n true` succeeds.
