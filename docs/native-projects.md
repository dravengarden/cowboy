# Native projects and AI placement

Cowboy owns Projects, their Machines, installed AI Plugins, and session
placement. Stormbird supplies connectivity. Matrix directories, SSH aliases,
`mx`, and copied routing instructions are not part of this workflow.

New Session selects **Project**, then **AI installation**. A choice such as
`Hawk / columbus / cowboy` has the stable identity `(hawk, cowboy)`; its label
only controls presentation. `Claude · OVH` selects the installed, ready Claude
Plugin on OVH. Cowboy derives Remote when the two Machines differ, and Local
when they match. Remote candidates additionally require the exact Provider
generation to accept the target executor. Missing connectivity, installation,
authentication or executor compatibility never falls back to local execution.

The existing signed Codex and Claude Plugins keep their native runtime and
authentication on the AI Machine. The target Machine owns the worktree and
execution keeper. No separate remote Plugin is installed. Cowboy creates its
private runtime entry automatically; the operator need not create a matching
directory on OVH. Provider installation and login retain their existing owners.

## Project owner

Settings → Machines → Details → AI and projects manages the selected Machine.
Registration, rename, removal and bounded discovery go through a closed core
Machine command, protocol 24. The target validates and canonicalizes a directory
before registering it. Changing the path under an existing ID is refused;
register a new identity for a moved project. Labels may contain `/` for grouping.
Discovery examines at most 1,024 nearby directories, three levels deep, returns
at most 200 candidates, skips symlinks and common build/cache directories, and
does not register candidates automatically.

`<Machine state-dir>/projects.json` is the authoritative registry after the
first successful edit. Until then, the existing host workspace file and CLI
roots supply bootstrap inventory. The first edit imports that entire inventory,
then Cowboy alone owns it. Later host configuration changes cannot resurrect a
removed alias. A malformed managed registry fails closed. Updates use the
observed revision, private atomic replacement, file and directory fsync, and the
existing workspace-identity retirement lock. A stale or interrupted response
requires reading the registry before another edit; clients do not retry effects.

Machines advertise `workspace_owner` together with the revision. Host deployment
health verifies Cowboy's registry observation after adoption, and the exact Nix
bootstrap roots before it. Updating host configuration does not replace a
Cowboy-owned registry. An explicit `project-adopt` operation can transfer the
unchanged bootstrap inventory, including an empty registry, to Cowboy.

Removing a project removes admission for new sessions. It does not delete source
files, worktrees, branches, history or a running session's execution binding.
Git projects retain fetched-default-branch worktrees; unavailable configured
remotes fail closed. Non-Git directories retain the existing shared-in-place
behavior, explicitly described in the picker and registration UI.

## Runtime policy

Cowboy Service persists `<data-dir>/project-placement.json`. Each enrolled
Machine can offer projects, run AI locally, remotely, in either mode, or disable
new AI placement. Remote policy may restrict target Machines. One Machine can
be preferred for AI; this is a preference among compatible installations, never
a fallback around a denied policy. If that Machine is unavailable, the user
must explicitly choose a different installation. `COWBOY_EXECUTION_RUNTIME_MACHINE` is only a
bootstrap preference until a policy is saved, no longer the feature switch.

Both enrolled-Machine creation endpoints enforce policy independently from the
picker. Existing sessions retain their original bindings when policy changes.
The legacy caller-owned local API workspace contract remains unchanged.

All mutations require a Product Operator or the existing explicitly delegated
local host Operator. Project requests are bound to the Service, Machine and
original authenticated connection. An older Machine rejects registry requests
before dispatch; its advertised roots remain usable until it is upgraded.

Examples on a Controller with host delegation already enabled:

```sh
cowboy operator machines
cowboy operator projects --machine hawk
cowboy operator project-discover --machine hawk --root /home/draven/columbus
cowboy operator project-register --machine hawk --revision <observed-revision> \
  --id cowboy --name columbus/cowboy --path /home/draven/columbus/projects/cowboy
cowboy operator project-policies
cowboy operator set-project-policy --machine ovh --file ovh-policy.json
```

The policy update file includes `expected_revision`, `preferred: true`, and
`policy: {agent_mode: "remote", hosts_projects: false,
remote_targets: ["hawk", "falcon"]}`. Read the resulting receipt. No browser
credential, live database edit, or installation-pointer write is needed.

## Matrix migration and release boundary

Upgrade Controller, Web and the participating Machines through their component
owners. Preserve workers through the separate Machine maintenance boundary.
Verify real target registrations before removing OVH aliases; rename only
presentation labels and retain existing target IDs. Set OVH to prefer Remote
with Hawk/Falcon targets and disable OVH's project offering. Remove the retired
OVH registrations through the registry API. Keep old Matrix source directories
and task worktrees until their owners finish or explicitly migrate their work.
An already-running legacy conversation is not silently rebound or replayed.

Managed registries require protocol-24 readers. Any later rollback must be a
descendant release retaining this reader; an old Machine binary would read its
obsolete bootstrap inventory. The active execution binding reader/recovery
floor remains unchanged. Provider packages and subscription authentication are
not upgraded by this core feature.

Verification includes registry CAS/restart/retirement, policy enforcement,
identity-independent grouping, and the real enrolled-session fixture with
`native_projects: true`. The fixture uses a signed scripted Agent and no model
requests; it tests transport and lifecycle, not subscription inference or WAN
latency.
