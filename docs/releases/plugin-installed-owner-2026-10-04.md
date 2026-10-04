# Installed Cowboy transaction owner — October 4

The user selected retaining sudo rights. Human/Agent administrator access stays
trusted; independently elevated old tools are outside the selected guarantee.
Supported component dispatch and failed-transaction repair use the installed
machine owner, rather than selecting a caller-built privileged implementation.

Columbus implementation `57985775` removes the caller-executable option and
self-executable default. Public dispatch first resolves
`/run/current-system/sw/bin/cowboy-release-activate` to an executable regular file
at an exact immutable `/nix/store/<root>/bin/cowboy-release-activate` path. The
resolved identity is carried into the detached unit; a later host-link replacement
does not redirect it. Missing, mutable, dangling or incorrectly shaped owners
refuse without falling back to the caller. This is a bounded dispatch snapshot,
not proof that no administrator can replace host policy or invoke another binary.

The saved `candidate` transaction entrypoint now refuses before Git fetch, Nix
build or privileged dispatch. Controller failed-rollback recovery defaults to
`installed`, matching the other component recipes. An owner missing required
compatibility/repair behavior must first be upgraded by its owning host release.
The public installed owner still accepts typed maintenance and recovery choices;
its existing locked ancestry, journal, reader-floor and rollback checks remain.

Pinned-shell `just verify`, all Machine Go packages with `go test -race ./...`
and `go vet ./...` passed. Tests cover canonical/mutable owner paths, missing
and dangling installations, selection retained after replacing a temporary
installation link, and four retired-entrypoint argument forms with Git/Nix
tripwires. The positive selection test ran against the existing immutable host
owner; non-Nix test hosts without one explicitly skip that test. Four actual
owning `just` recipes also refused `candidate` with exit 2; none dispatched a
transaction. No bad production journal or failed Machine transaction was seeded.

The final owner source `e4a2b363503ade67576752382cf140d08686a2fb` integrates
fresh main, active source `8ad13670`, earlier concurrent native qualification
records and the new SideStore source-icon update `0389ce4e`. The full repository
gate passed again for the merge before committing and building. Source-icon
integration is not an iOS build or a newly shipped IPA.

The clean owning Hawk build produced
`/nix/store/jcrnypw11ammw2hhdiv0yrq0jic97ghb-nixos-system-hawk-26.05.20260731.5b4f72e`.
Activation succeeded, published, as transaction
`1791076942994350820-e4a2b363503a` at `2026-10-04T09:22:25+08:00`.
Required host health checks passed with no new failed units. No unrelated failed
unit was cleared. The installed activator is
`/nix/store/a4jpv2prm56mvkcfy8f0w969hdm4srmc-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`,
SHA-256 `9e1294320f0242363dc426aa856b602c3e55816a250544334e2a353bae545f8f`.

Recursive system/user unit comparison found `mandb.service` and system-path
changes in AccountsService, D-Bus and polkit drop-ins. Cowboy/worker definitions
were unchanged. The actual switch journal records AccountsService stopped and
started, polkit restarted, D-Bus reloaded and NixOS user activation units
restarted. The receipt's top-level changed-unit and explicit-restart lists are
not complete process-change evidence.

Samples at `2026-10-04T01:22:06.748Z` and `2026-10-04T01:23:20.345Z` retained
all 13 ACP worker and three keeper PIDs, Machine PID `1928418` and Controller PID
`486493`. Sixteen worker/keeper processes were present before this release;
these samples do not assert continuity for a previously present fourth keeper.
The Machine component receipt, reader-floor bytes, resolved Web target and SPA
version `798bda6db1a3a8958a6102125058e8e2` were unchanged. HTTPS
health/version/SPA/SW/deployment-health returned 200, HTML/SW kept `no-store`,
and Machine stayed connected with generation `worker-6ede7a91cc8b8b3402d4`.
Deletion state remained only `.lock`; no Machine component journal remained.
These are bounded process samples, not native-generation swap/resume acceptance.

The sudoers SHA-256 was unchanged before/after:
`149c822dfd64e9b5354c33e050f27b6f8da51779c05c2186728a37a0862eaf69`.
The user's retained administrator boundary is recorded in the
[authority audit](../plugin-activation-authority.md). No role separation,
permission revocation or portable installation migration was performed.

The [machine-readable evidence](../experiments/plugin-installed-owner-2026-10-04.json)
records the immutable owner, host receipt, actual retired-recipe refusals,
recursive unit differences, switch journal, sudoers hashes and process samples.
Portable compatible-reader admission, production deletion writing,
cross-generation recovery and writer-release acceptance remain closed.
