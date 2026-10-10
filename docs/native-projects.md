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

Clicking a registered parent such as Hawk / columbus expands its children,
selects the parent by default, and keeps the picker open. The current directory
has a distinct outlined row, open-folder icon and caption above its children.
Selecting a leaf closes the picker. Groups without a registered project only
browse. Keyboard Right/Left browses levels; Enter on a parent expands it. The
closed AI field shows one icon and name on a single row, while its menu retains
mode and vendor details.

Machine `schedulable` reports connected AI runtime capacity, independent of
whether that Machine hosts projects. Both Controller snapshots and browser
session-occupancy projection preserve this distinction. An AI-only Machine with
an empty registry remains available; offline, draining and full runtimes do not.

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

The Controller host can additionally restrict exact Provider IDs with repeated
`cowboy serve --provider-runtime-machine PROVIDER=MACHINE` arguments. For
example, `--provider-runtime-machine codex=ovh --provider-runtime-machine
claude-code=ovh` permits those Providers only on OVH, including after other
Machines enroll. Provider variants retain independent identities and policies.
Multiple entries for one Provider form an allowlist. This host restriction
intersects project placement; an editable preference cannot override it.

Admission covers the picker, both creation APIs, legacy local creation,
restored worker launches and new prompts, temporary login executors, and exact
Plugin usage commands. An unavailable permitted Machine produces an error;
the Controller does not choose a disallowed Machine. Existing turns are not
interrupted or relocated. Native Cancel and explicit deletion remain available.
For a denied historical placement, forced restart returns an error after sending
Cancel and leaves the session state intact. The retained Machine protocol has no
non-destructive hard-stop-only command; a wedged worker requires process
maintenance on its original Machine. Never substitute Session deletion for that
operation. Native processes that were already running can continue background
work, so inspect existing sessions before activating a new restriction.

The arguments are startup configuration, not a new persistent schema or Machine
wire field. Keep them in the machine-owned service definition. An older
Controller refuses the unknown argument; a recovery binary must support the
same restriction before it can replace an enforcing Controller. Removing the
arguments is an explicit host-policy change. This does not fence manually
launched CLIs, shell tools, credential replicas, or a user's browser network.

Native Codex owns its processes and remote execution protocol; it cannot select
Cowboy's enrolled Machines for another Provider's login or usage command.
Cowboy's existing Controller admission is the narrow extension for that
cross-machine gap. Claude uses the shared admission with its native Mods
adapter. Delete this guard when the underlying runtime transport can enforce
the same per-Provider restriction for sessions, login and account operations.

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

## Production activation, 2026-10-02

Cowboy `89bf4b72` is active on the Controller, Web, and Hawk/Falcon/OVH
Machines. Columbus `6a145781` supplies the accepted Hawk/Falcon cold readers
and OVH's versioned host override and shared guidance. The
[release receipt](experiments/native-projects-release-2026-10-02.json) records
the component transactions, actual recovery artifacts, tests and final policy.

Cowboy owns 30 available Hawk projects and 25 Falcon projects, grouped under
`columbus/...` and `suger/...`. Target validation found four absent Hawk
bootstrap directories and nine absent Falcon directories, including Matrix;
their new-session entries were retired without deleting files. OVH advertises
zero projects and is the preferred Remote-only AI Machine, permitted to target
Hawk and Falcon. Both targets report ready `Codex · OVH` and `Claude · OVH`
installations. Existing Provider packages and subscription authentication were
retained. No new model request was made for release acceptance.

The OVH maintenance retained all three original worker processes at acceptance.
Historical Matrix directories and sessions keep their original ownership;
start a new session to use native Project placement. Hard-reload the PWA for
the new picker; a WebSocket reconnect alone retains the previous JavaScript.
Project registration and Machine mode settings now live under Settings →
Machines → Details → AI and projects.

All 13 native-session checks passed for the standalone release, actual Hawk
cold outputs and actual Falcon cold Machine. The Firefox picker fixture passed
seven checks; the Rust, Web, PostgreSQL, populated reader and startup gates
also passed. Deployment observation caught an OVH heartbeat timeout followed
by automatic reconnection of the unchanged Machine process. Its cause was not
established in this change. The final readiness checks passed, but this receipt
does not establish long-duration connection stability, production model
inference, physical iPhone behavior or WAN/token/turn parity.

The same-day picker follow-up, Cowboy `09338043`, fixes the Controller and
browser occupancy projections that still required an AI Machine to host local
projects. The earlier picker fixture bypassed that occupancy projection and
missed the production failure. Its replacement exercises both Codex and Claude
through the real projection; the enrolled-session fixture now checks the actual
browser Machine inventory after removing every runtime project. Controller and
Web (`cowboy-v1792`) activation succeeded, with OVH online, schedulable and still
hosting zero projects. The existing Machine generation and project registries
were retained. The follow-up receipt (in Git history)
records the failing old behavior, passing regressions, and live Provider joins.

Web `612aaf14` (`cowboy-v1793`) adds the compact AI selection and direct parent
project selection described above. Its activation receipt (in Git history)
records 17 browser checks, 1,980 Web tests and the unchanged Controller process.
