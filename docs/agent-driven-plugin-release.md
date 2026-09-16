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

Browser authentication remains useful for remote operators. A Controller host
also has an operating-system authority boundary: its Service account controls
the application and its data. The local Operator endpoint makes that authority
explicit and auditable for Plugin operations.

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
  --machine hawk --plugin claude-code --version 3.1.24 \
  --digest sha256:f3f0b9245fcd7084b41c4d3073206926cf0d8b9db0c9badaf3ed8186aca37919 \
  --operation-id hawk-claude-code-3-1-24-reviewed-upgrade
cowboy operator operations --machine hawk --plugin claude-code
cowboy operator inspect --machine hawk
cowboy operator usage --refresh anthropic
```

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
  listener and revokes its process-local grants.

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
saved-ID observation across restart without replay. It uses disposable state
and identities, never production credentials or a real agent session.
