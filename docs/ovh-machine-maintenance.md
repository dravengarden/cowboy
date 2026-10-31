# OVH Machine host maintenance

OVH is an Ubuntu Device without Nix tooling. Its Machine runs as the `ubuntu`
user service `cowboy-machine-svc-4e4d5154f3df9aa109d7d841dd925fd7.service`,
whose drop-in `10-plugin-admission.conf` pins two separate things:

- the Machine host: `ExecStart=/nix/store/<hash>-cowboy-machine-release/libexec/cowboy-machine`;
- the retained worker bundle: `--worker-command` and `--desired-generation`.

A host-only release replaces the first and keeps the second, so detached ACP
workers keep running and reattach. `--workers` additionally selects the
release's own worker bundle: idle workers adopt it after the new host connects,
and busy ones at their native safe boundary. Use it when the worker contract
changed (for example a Provider SDK the old worker cannot read).

## Release

Build a plain `.#cowboy-machine-release` from a clean commit already on
`origin/main` (the maintenance script admits only that store-path shape and its
provenance), then run on Hawk from the repository root:

```bash
nix build .#cowboy-machine-release --out-link result-machine
tools/ovh-machine-host-release.sh "$(readlink -f result-machine)"
# or, also rolling out the release's worker bundle:
tools/ovh-machine-host-release.sh --workers "$(readlink -f result-machine)"
```

The driver reaches OVH only through the `ovh` SSH alias and:

1. refuses a dirty or unpublished release, or a drop-in without exactly one host
   `ExecStart`;
2. refuses while the Machine is driving a Plugin installation (a pending
   install attempt younger than 15 minutes): a host restart during staging
   leaves an interrupted attempt that blocks that Plugin's usage, generation
   retention and upgrades until it is reconciled. Retry after it finishes;
3. copies only the store paths OVH lacks, verifies the archive checksum, and
   records the full closure under `/nix/var/nix/gcroots/ovh-cowboy-machine-<rev>`;
4. writes a root-only receipt `/var/lib/columbus/ovh-machine-<rev>-<time>/`
   with `previous.conf`, a `candidate.conf` that differs only in the host
   release and its header revision, the current host identity and every live
   `cowboy-acp-worker` identity;
5. arms an independent root timer that rolls back after four minutes, then runs
   `activate` in its own root unit. The script switches the user unit to
   `KillMode=process` so stopping the host does not stop workers, and refuses if
   any recorded worker changed;
6. waits for the new host to log `Machine controller authenticated`, then runs
   `accept` (which rechecks every worker and restores normal containment) and
   disarms the timer.

With `--workers` the candidate also replaces `--desired-generation` and
`--worker-command`, the receipt records each live worker's full session
identity and `worker-rollout.json`, and the modes are `activate-workers` /
`accept-workers`. Acceptance requires every original session to keep its
exact worker or to have exactly one replacement on the new generation with the
same session, socket, cwd and Provider generation; it is retried within the
rollback window while idle workers are being replaced.

Before the receipt records live workers, `--workers` hibernates every idle
(`running`) OVH session itself, default or named Provider account, through
`tools/ovh_machine_idle_sessions.py` and this computer's `cowboy login`
(`cowboy session list --json` is unpaged; `cowboy session hibernate` is the
Web UI's own refusal-checked action). It waits until the Controller reports
each one `exited`, so a late stop cannot read as a retained worker changing.
A hibernated session has nothing to replace in the rollback window and resumes
on the new generation when next opened. A session that is busy, or that the
Controller/Machine refuses because work started meanwhile, stays retained and
under the unchanged acceptance rule; crashed and dormant sessions are not
touched. The receipt's `hibernation.json` lists both sets. Do not substitute a
hand-built list: on 2026-10-31 a sweep over the first page of Cowboy MCP
`sessions.list` (default 50, which then returned no truncation signal) missed
two retained sessions, and both rollouts were rolled back.

Acceptance deliberately does not excuse a retained session whose replacement
failed for a session-local reason, such as an expired Provider login: process
inspection on OVH cannot tell that from a rollout fault, and excusing it would
weaken the proof that every original healthy session keeps or replaces its
worker. Hibernation removes idle sessions from that exposure; if a busy one
still fails, the timer rolls back, the broker falls back to that session's
previous generation where the failure permits (named accounts included), and
the operator repairs the session (for example its login) before retrying.

If verification fails the driver exits and the timer restores `previous.conf`.
The receipt keeps `awaiting-acceptance.json`, `committed.json` or
`rolled-back.json`. The SSH session that runs the driver may itself be a Cowboy
session on OVH and briefly lose its connection while the host restarts; both
systemd units are independent of it.

`tools/ovh_machine_host_maintenance.py` is the canonical copy of the script
installed into each receipt. Change it here, not on the host.
