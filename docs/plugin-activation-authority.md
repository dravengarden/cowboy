# Independent activation authority: audited gap and proposed boundary

This records the authority audit, the selected administrator boundary and the
remaining stronger isolation proposal. It is not a permission fence. At the audit baseline, the
owner at Columbus `8ad13670` validated ordinary rollback targets; it did not
control independently privileged old activators or same-user old installers.
Production Session deletion writing remains disabled.

## Actual bypasses

At the audit baseline, the Columbus dispatcher resolved its own executable and
supplied it to `SystemdRunArgs`, then invoked `sudo systemd-run` to execute that binary's
internal transaction as root. An independently supplied old executable can
therefore run its own transaction implementation rather than the installed
owner's admission checks. Its application code need not read the current reader
floor. See `machines/internal/cowboyrelease/release.go` in Columbus, `Run` and
`SystemdRunArgs`, and the root requirement in `transaction.go`.

Hawk's `machines/hawk/nixos/configuration.nix` explicitly permits `draven` to
run `ALL` commands as root with `NOPASSWD` and `SETENV`. The Cowboy service
module intentionally runs Controller and Agent workers as that human user and
includes the privileged sudo wrapper in the Agent runtime path. This is an
existing host access policy, not an accidental missing file mode. Restricting
one activator command while retaining `sudo ALL` cannot remove the bypass.

Portable installation writes bootstrap payloads, configuration and the selected
launcher under the caller's state and home directories. The current installer
checks compatibility; the old installer does not. The independently supplied
old-installer negative controls in
[bootstrap compatibility acceptance](releases/plugin-bootstrap-guard-2026-10-03.md)
and `tests/bootstrap_refresh_releases.rs` prove it can replace the guarded
bootstrap/launcher while preserving a synthetic committed record. These are
previously recorded disposable tests, not new production experiments.

Changing a command symlink, adding a CLI check, a same-user lock or a same-user
floor does not fence a separately invoked older executable. Removing an
executable bit cannot revoke another immutable copy. A protected file beneath
caller-replaceable ancestors is not a protected installation identity either.

## Concrete implementation if strong actor isolation is selected

1. Establish a distinct deployment submitter identity without unrestricted
   sudo, membership granting unit administration, or write access to the owner
   namespace. The owner retains privilege; human administration is a separate
   explicitly trusted role. Running both roles as `draven` with `sudo ALL`
   cannot establish this boundary. Separating only the CLI name is insufficient.
2. Put the release transaction behind a machine-owned privileged entrypoint.
   Accept only the immutable release root, explicit maintenance authority,
   optional accepted recovery root or exact failed-transaction ID, and an
   operation identity. Select the host/lane contract on the owner. Do not accept
   an executable, shell command, unit properties, environment or caller-selected
   state/profile paths. The owner always executes its installed immutable
   transaction implementation and enforces the existing shared deployment lock.
3. Restrict profile, receipt, floor and installation-pointer writes to that
   owner. Current root-owned component paths cover unprivileged callers already;
   the missing piece is their unrestricted elevation. Retain the host build and
   activation boundary; deploying an older host owner requires compatible
   owner admission and an explicit, audited administrative recovery decision.
4. For portable installations, introduce a separately administered installation
   anchor outside caller-replaceable ancestors and a fixed system-owned launcher
   or supervisor. Old same-user installers can alter their legacy directories
   but cannot replace the supervisor's selected installation or reader floor.
   Define Linux and macOS paths and rights independently. A user-owned launch
   agent or user systemd unit does not itself provide this separation.
5. Enroll/migrate explicitly, verify selected binary and state identities before
   admitting writes, and preserve old directories and all unresolved evidence.
   Never infer enrollment from a missing namespace or automatically replace a
   running Machine, worker generation, Provider account or active session.

The residual host gap is privileged installation/deployment authority and
cross-version state admission. Native Codex owns tasks, worktrees and execution;
this owner must not duplicate those capabilities or create a generic executor.
Claude follows through the same closed deployment client. The local layer's
deletion trigger is a platform-owned equivalent with the same rights and
cross-version admission guarantees.

## Alternative with the current administrator policy retained

Keep full human/Agent `draven` access and document it as trusted administration
outside the enforceable owner guarantee. Route supported clients through the
installed owner, but do not claim independently root-capable old executables
are fenced. Keep same-user portable installations read-only for this dataset;
a signed reader declaration alone cannot admit a writer there. This alternative
does not close the strong independent-old-tool completion requirement.

## Acceptance before any writer admission

Use actual immutable old and new executables under the intended submitter
identity. Prove old ordinary activation, direct internal transaction, profile
write, transient-unit dispatch and portable replacement cannot change the owned
selection, floor or supervisor. Prove the new closed owner request succeeds,
while arbitrary executable/unit/environment/path requests refuse before effects.
Include principal changes, concurrent old/new attempts, crash/reopen, exact
compatible rollback and owner-update/recovery cases. Preserve failed evidence.
Administrative emergency access must be separately classified and audited;
root access cannot be described as revoked while it remains granted.

Do not test these refusals by pointing old privileged tools at production.
Disposable process fixtures are required before a host rights change. Actual
production acceptance must separately record health, receipts and retained
session/process observations; equal PIDs alone do not prove generation resume.

## Selected scope — October 4

The user explicitly selected retaining sudo rights and continuing. The existing
human/Agent `draven` administrator access stays trusted and unchanged; strong
actor isolation is not selected. No further permission decision is pending for
routine work within this scope.

Supported component deployment and failed-transaction repair must use the
installed machine owner. Caller-built clients must not elevate their own
transaction executable; the old `candidate` transaction choice must refuse.
The installed executable must be resolved once to its immutable Nix identity
before preflight, rather than dispatching through a mutable host symlink.
Updating that owner follows the existing clean committed host release path.

This does not revoke independently invoked root-capable old executables or
same-user portable installer writes. They are outside the selected guarantee,
not secretly made safe by another application-level check. Portable compatible
reader/recovery admission, production deletion writing and actual writer-release
acceptance remain separate and closed until their own requirements are met.
The whole refactor remains open.

The [installed-owner release](releases/plugin-installed-owner-2026-10-04.md)
implements the selected supported-entrypoint policy and records successful
Hawk activation at owner source `e4a2b363`, with unchanged sudoers hashes and
bounded process continuity. Strong actor isolation remains unimplemented.
