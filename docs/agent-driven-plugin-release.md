# Agent-driven Plugin release and upgrade

Cowboy separates publishing an immutable release from installing it on a
registered Machine. Agents with delegated host authority can perform both
through owned commands, without copying a browser session or editing live
installation pointers.

## Why the previous upgrade stopped

Claude Code 3.1.24 was signed, published and verified, while Hawk still had
3.1.14 installed. The attempted management requests returned HTTP 401 because
the agent had host access but no browser Operator credential. Those requests
did not enter the installer. A successful Catalog publication is not an
installation receipt.

The [completed Hawk rollout](releases/agent-operator-upgrade-2026-09-16.md)
subsequently installed 3.1.25 and verified real subscriber quota, including a
collector correction discovered during the first live upgrade.

Browser authentication remains useful for remote operators. A Controller host
also has an operating-system authority boundary: its Service account controls
the application and its data. The local Operator endpoint makes that authority
explicit and auditable for Plugin operations.

## Admission is per Machine

Publishing a release makes it installable; it does not make every Machine able
to accept it. A Machine only reports an observable installation target when its
own agent runs with `--plugin-operation-admission`, and that flag is a host
recovery-contract decision taken per machine, not part of a release.

Hawk passes it. Falcon and macbook-air do not, so every install to them is
refused before dispatch — including a same-bytes reinstall of the version they
already run, which is how to tell this apart from a release problem. The
Controller now names that refusal: "the Machine did not report an observable
installation target." Enabling admission elsewhere is separate host maintenance
(Columbus `machines/docs/nixos-deployment.md`), not something an upgrade command
can do for you.

So a fleet convergence can legitimately end with Machines still behind. Report
them; do not retry under a new operation identity.

## Enable once, then operate

Use a Controller binary that includes `cowboy operator`. Run these commands on
the Controller host **as its Service account** (on Hawk, `draven`). The default
data directory is `/var/lib/cowboy`; use `--data-dir` for another instance.

```sh
cowboy operator enable
cowboy operator status
cowboy operator refresh-catalog
cowboy operator catalog
cowboy operator inspect --machine hawk
cowboy operator operations --machine hawk --plugin claude-code
```

`enable` installs a new host delegation generation. It is an explicit local
policy action, not a browser login. Repeat it only when intentionally rotating
the delegation: rotation invalidates previous in-flight approvals. The grant
survives Controller restarts; captured in-flight approvals do not.

Review the exact signed release and installed target before submitting:

```sh
cowboy operator upgrade \
  --machine hawk --plugin claude-code --version 3.1.25 \
  --digest sha256:88e7b0d1a832102e09fc1d2cc3f968a831046c357a89d7d59b1977792204d1f9 \
  --operation-id hawk-claude-code-3-1-25-reviewed-upgrade
cowboy operator operations --machine hawk --plugin claude-code
cowboy operator inspect --machine hawk
cowboy operator usage --refresh anthropic
```

## Converging instead of typing digests

One Plugin at a time is the exact-release path above. To bring whole Machines
up to the Catalog's newest ready releases without authoring a digest:

```sh
cowboy operator converge --machine hawk --machine falcon
cowboy operator converge --machine hawk --machine falcon --apply
```

The first form is a dry run and prints the plan. The `--machine` order is the
rollout order and its first entry is the canary: the next Machine is attempted
only after the previous one's re-read inventory proves it converged, so a bad
release reaches one host rather than the fleet. With no `--machine`, every
connected Machine converges in registry order; `--plugin` bounds the run.

For a Machine declared in
[the Service-side membership document](machine-plugin-membership.md),
convergence also installs the declared Plugins it lacks and removes the ones it
should not have. Removal takes the ordinary uninstall preview first and refuses
as soon as that preview names an affected session, so a live conversation is
never ended to satisfy a list.

Targets come from the Catalog, so no digest is typed. A release that is not
`ready`, or that does not declare that Machine's platform, is not a target. A
Plugin holding an active session lease is reported rather than recycled under a
live worker. An installed version ahead of the Catalog is reported, never
downgraded. A Plugin the Machine does not already run is never installed —
that is a separate decision. Each upgrade carries one deterministic identity
(`<machine>-<plugin>-<version>-converge`), so repeating a run after a lost
response observes the saved result instead of installing twice.

Convergence runs the same durable installation transaction, the same signed
Catalog resolution and the same host delegation as a single upgrade. It is not
a second installer, and it cannot install a release the Controller would refuse
by hand.

`cowboy operator freeze --reason "..."` stops it, together with the
Controller's own component convergence; `unfreeze` resumes. A frozen Service
refuses `--apply` and still allows a dry run. Freezing cannot undo an effect
that already happened — dispatched work finishes under its own transaction.

The CLI prints JSON with an HTTP status, operation ID and response data. HTTP
204 means the request completed; inspect the saved result and installed
inventory for acceptance. HTTP 409 can return an already saved result, with
`execution_authorized: false`. A failed response produces a nonzero CLI status.

There are no automatic HTTP retries, redirects or proxy discovery. If a
connection closes or times out, inspect the original operation before choosing
another identity. Repeating the same ID cannot replay the installation. An
unknown outcome retains its slot fence and requires reconciliation; a new ID
is not a recovery mechanism.

To revoke host delegation, including future effect checkpoints:

```sh
cowboy operator disable
```

Already dispatched Machine work can finish under its existing finite lease.
Disabling the grant cannot undo an effect that has already happened.

## Authority and implementation

- The endpoint is a private Unix socket at
  `<data-dir>/local-operator/control.sock`, in a Service-owned mode-0700
  directory. Socket, lock and grant files use mode 0600. A missing, malformed,
  linked, foreign-owned or permissive grant fails closed.
- The server accepts the Service UID or root from kernel peer credentials.
  Request headers, cookies and JSON cannot supply this identity. The CLI is
  deliberately run as the Service account so ownership checks also apply when
  enabling or disabling delegation.
- Only a closed set of Plugin catalog, inventory, installation, receipt and
  usage operations is exposed. These routes are not mounted on the TCP router.
  This does not authorize an arbitrary shell, remote HTTP proxy, user-account
  administration, telemetry selection or session termination.
- Installation uses the same `OperatorApproval`, exact signed Catalog resolver,
  original Machine connection, target observation, finite coordinator and
  Service/Machine journals as browser installation. No alternate installer or
  private database mutation is added.
- Approval captures the grant inode and random generation. Each existing effect
  checkpoint revalidates it and the listener lifetime. Removal, replacement,
  invalid permissions or shutdown closes further effects. Observed authority
  failure is permanent for that bound operation. Its original five-minute
  monotonic budget is never renewed by waiting or reconnecting.
- Durable evidence uses `Admin { account: "unix-uid:<uid>" }` in the existing
  bounded actor codec. Browser admin names cannot contain `:`, so they cannot
  claim this namespace. This evidence does not create an admin account or
  reconstruct a credential after restart. Host approval cannot become a
  Product login.
- An exclusive endpoint-owner lock prevents a second Controller from unlinking
  the first one's socket. Cleanup removes only the socket inode owned by that
  listener and revokes its process-local grants. The guard explicitly unlocks
  the shared file description on teardown and failed binds: an inherited
  descriptor between fork and exec must not retain ownership after the listener
  ends. Closing that old descriptor later cannot unlock a successor's lock.

The delegation covers the Controller's registered Machines and trusted Plugins.
All agents running as the Service UID share this authority; it is not per-task
isolation. Repository write access under another UID alone is insufficient.
Use a separately scoped remote identity if future agents need a narrower
boundary instead of sharing the Service account.

Linux documents pathname permissions and `SO_PEERCRED` in
[unix(7)](https://www.man7.org/linux/man-pages/man7/unix.7.html). The pinned
client uses reqwest's
[Unix socket transport](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.unix_socket).

## Agent release procedure

1. Prepare and verify a committed Plugin release with the canonical
   [release skill](../.agents/skills/release-cowboy-plugin/SKILL.md). Publish its
   signature, package, host bundle and immutable runtime artifacts; verify their
   public hashes. Keep the receipt.
2. When core changes are needed, integrate remote main, build the narrowest
   immutable component and activate it with the host-owned activator. A
   Controller restart is separate from Machine maintenance.
3. With upgrade authorization and host delegation, inspect Catalog, Machine
   inventory and any outstanding operation. Submit an exact release with one
   saved operation ID through `cowboy operator upgrade`.
4. Inspect the durable result, installed version/digest and account usage.
   Preserve running sessions bound to earlier generations. Report those
   sessions separately from the Machine's newly installed default.

## Verification

The normal gate covers private file/peer checks, grant rotation and revocation,
listener ownership, CLI requirements and the existing installer failure paths.
For the actual CLI-to-Controller-to-Machine path, run:

```sh
nix develop -c just plugin-local-operator-conformance /absolute/matrix.json /absolute/new-receipt.json
```

It reuses the immutable Controller/Machine matrix format documented in
[the installation journal](plugin-install-journal.md). Only the supplied active
pair executes this test; this is not a replacement for the reader-floor matrix.
The isolated fixture proves default-deny, real peer identity, TCP separation,
one exact signed Machine installation, durable host attribution, revocation and
saved-ID observation across restart without replay. Its byte-preserving relay
accepts installation-compatible protocols 19–21 only within the Machine's
advertised range, records the negotiated protocol, and refuses missing Hello,
older or unknown future protocols. It uses disposable state
and identities, never production credentials or a real agent session.
