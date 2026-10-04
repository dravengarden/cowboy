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
