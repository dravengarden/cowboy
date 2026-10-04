# Restore portable selection to its retained floor anchor — October 4

Losing either selected-host pointer previously prevented startup and signed
refresh even when the portable floor and original signed anchor were intact.
Clearing that floor or selecting an arbitrary bootstrap is not an accepted repair.

The Machine-host installer now exposes an explicit offline recovery command:

```sh
cowboy-machine-install --restore-floor-selection \
  --state-dir /absolute/machine/state \
  --artifact-public-key /absolute/publisher.pub
```

It checks the uncommitted deletion namespace, private bounded floor, canonical
state/publisher binding, retained original signature and proof hash, exact artifact
and payload bytes, regular selection directories and executable access. It selects
only the anchor recorded by that floor. Existing pointers must already resolve to
that exact generation/executable; other targets, dangling pointers, regular files
or linked directories refuse before any write. A missing anchor refuses rather
than accepting a new package or erasing evidence. The key is a bounded regular
no-follow file, and no publisher code or Controller request is executed.

After preflight it fills missing `active/machine_host` and
`commands/cowboy-machine` symlinks exclusively, never overwriting either pointer.
Missing selection directories are created mode 0700. It reauthenticates anchor and
floor before each publication, compares exact floor bytes and canonical target,
syncs directories and runs the ordinary read-only cached-host startup check.
An interruption between links leaves startup fail-closed; explicit repetition can
complete that partial state or verify an already restored selection. The command
prints a closed read-only admission result only after the final check passes.

The command does not rewrite launchers, bootstrap generations, identity, token,
floor, anchor proof or payloads, and does not choose the most recent inventory
entry. Restoring the retained first anchor is an explicit administrator decision.
It does not fetch/rebuild a missing anchor, replace a damaged or different existing
selection, admit committed deletion records, rotate keys or enable the writer.
The two-link update is not globally atomic or a full power-loss recovery proof.
Concurrent same-user/admin mutation is outside this authority boundary, and the
administrator's sudo rights remain intact. Installers without the Machine-host
feature refuse this command.

## Verification and production receipt

Source/native checks and production receipts are appended after acceptance.
The release retains the separately accepted worker bundle through
`cowboy-machine-host-release`; no worker pin or adapter generation changes.
