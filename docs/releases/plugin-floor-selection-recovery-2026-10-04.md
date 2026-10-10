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

The release retains the separately accepted worker bundle through
`cowboy-machine-host-release`; no worker pin or adapter generation changes.

Source gates at `f4566114` passed all-feature library tests (1,811 passed,
48 ignored), integrations, standalone Machine tests (500 passed, 11 ignored),
all-feature/default Clippy with warnings denied, Rust formatting and Plugin/Provider
checks. Independent mobile/desktop Web changes and their receipt were integrated
afterward; the Rust package derivations remained unchanged. Final immutable
source-boundary and worker-registry checks passed. The default Nix package passed
1,330 tests, with 27 ignored, without retry or disabled checks.

The exact final release passed twenty-eight native raw/archive recovery cases:
both links missing, either link missing, selection directories missing, different
selection, corrupt or missing anchor, regular-file pointer, linked directory,
linked artifact proof, invalid floor, committed namespace, non-executable host
and wrong publisher. The preceding installer refuses the command. Successful
repairs preserve exact floor bytes, repeat idempotently and pass the ordinary
native startup diagnostic. Every recovery/diagnostic leaves the publisher's
first-instruction execution marker absent; invalid selection evidence remains
without repair. Fixtures use disposable signers and do not claim production
recovery or real Provider inference.

Native regressions also passed the five-case floored refresh matrix, nine signed
startup cases, signed installation/offline invalid-floor refusal, twenty-four
cached-launcher raw/archive cases and current-versus-preceding cache-only guard
admission. A first dispatch of the earlier merged artifact was refused before
activation because main had advanced again. Production retained the previous
receipt. A fresh clean merge generated the final release with the same native and
retained programs; exact native acceptance was repeated before successful dispatch.
No freshness rule or authorization check was bypassed.

Published source `102bcbf73c02c2cdb0945d7a4b093e40d69b06ff` was activated from
`/nix/store/2rf1w7b7rps36fbn0141d4wd5sl81zcz-cowboy-machine-release` by the unchanged
installed owner. Transaction `1791103481773238652-102bcbf73c02` succeeded,
published and committed at `2026-10-04T08:44:55.032981093Z`, with maintenance
enabled and no recovery. Its predecessor is the signed-refresh release
`hm3h8lgjgdvqyvi7i1bgr5wm9v91wjfl`. The actual running native executable is
`/nix/store/bzqzn28i7g3q8csm2r42vzw425904kha-cowboy-machine-0.1.0/bin/.cowboy-machine-wrapped`,
SHA-256 `1a173866246c09aedea80ea8bdc7a687e731753d581c23f58cb47e15438a8949`.
The installer entrypoint SHA-256 is
`5b8fe1a8ed17efc2214df657e610fcba55432779a380241151fe69cef9a13c3a`.

Samples at `2026-10-04T08:44:28.458Z` and `2026-10-04T08:45:32.151Z` retain
all thirteen workers and five keepers with the same IDs, states and PIDs. Resident
Machine PID changed from `452169` to `706832`; its `/proc` executable/digest
match the artifact. All six retained companion paths/digests, default generation
`worker-9fce17441fdd1e8ca642`, Controller PID `959309` and receipt, Web
profile/version and root reader-floor bytes remain unchanged. These are bounded
process samples, not a new worker-generation swap or session-resume acceptance.

All five HTTPS health/version/SPA/SW/deployment-health observations returned 200;
HTML/SW retain `no-store`, and Machine is connected/online. Both component
in-progress files are absent, and no failed system/user unit was observed or
reset. The deletion namespace remains only `.lock`, no production portable floor
was initialized, and actual new startup reports zero deleted sessions with
`writer_enabled=false`. No live selection restoration, signed production bootstrap
publication, Plugin operation or Controller/Web restart was performed by this
slice. Lost-anchor recovery remains closed.

Sudo remains available and its policy SHA-256 remains
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`.
The machine-readable receipt (in Git history)
retains exact source, artifact, process, refusal and activation evidence.
