# Trusted OVH SSH development acceptance

The owner explicitly authorized OVH as a trusted peer with the ordinary Hawk
developer account, including passwordless sudo. This supersedes the isolated
workspace boundary in the [earlier acceptance](ovh-hawk-ssh-development-2026-10-01.md).
Columbus deployed Hawk source `45a0669d` and published its deployment/security
receipt in `a86499d3`. No Cowboy application release or Machine restart was
needed to change this native OpenSSH interface.

## Production interface

The permanent Machine remains `ovh`, Service
`svc-4e4d5154f3df9aa109d7d841dd925fd7`, in its default Service state directory.
Signed installed Providers were Codex 3.1.30 and Grok 3.1.25, both active with
current credential replicas and materialization. Zed 1.20.0 remained installed.
Official `cowboy serve-acp --machine ovh --workspace matrix` used the already
approved sender-constrained client; no cookie or credential file was copied.

The host-managed `ssh hawk` uses the dedicated OVH key, pinned host identity and
MagicDNS. Its target account is `draven`; both real Providers returned `draven`
and `0` from `id -un; sudo -n id -u`. Native PTY, Git and Nix also worked.
Agent forwarding remains disabled. The agent receives a logical alias and
task worktree path; it needs no address, geography, proxy or tunnel knowledge.
This is transparent access, not enforced topology secrecy or hostile-agent
isolation. A compromised OVH execution account inherits this Hawk authority.
The two hosts consequently share administrative trust; OVH's local workspace
sudo denial does not contain a process that can administer Hawk.

## Frozen task

Each Provider received an independent normal-user Git worktree of the same
taskboard fixture. All project reads, edits, commands, tests and commits were
required to execute through real `ssh hawk` tool calls in the pinned Nix shell.
The two tasks are CSV export and a separate repair of concurrent lost writes,
failed-write truncation and permissive new database permissions. Frozen tests
must not be edited; new regressions belong in separate files. No business file
or shared service is a test target.

## Codex: passed

Session `sess-1790783866447` confirmed `gpt-6-luna`, effort `max`, through
acknowledged ACP config updates. It used 16 actual SSH tool calls, including a
subsequent reconnect follow-up. The initial seven-test suite had five failures:
two missing-export cases and three storage failures. It produced two separate
remote commits:

- `221f02d5` — CSV export and new edge-case tests.
- `c302a7e` — atomic, cross-process-safe storage and regression tests.

The full suite passed 11 tests. Independent operator tests then passed 100 CSV
records with Unicode, quotes, commas and embedded newlines; 144 concurrent
writes; 408 complete JSON reads during mutation; 12 interrupted-writer checks;
preservation of mode 0640; new mode 0600; read-only export; and corrupt-input
rejection. Frozen test hashes remained unchanged and the Git worktree was clean.
The original ACP prompt returned `end_turn` normally. After reconnect, a second
turn rechecked both commits and reran all 11 tests without repeating mutations.

The [machine-readable receipt](../acceptance-results/ovh-trusted-ssh-2026-10-01.json)
links tool commands, actual test output, installation inventory and a complete
Git patch series. Applying that series to an empty repository reproduced final
tree `a9a3d36a301a6a9f0bf9ba32d0bd1d466d2da580`. To rerun the independent holdout,
enter the reconstructed fixture and run `nix develop -c python3 PATH/holdout.py
./taskboard.py`. These artifacts omit reasoning, credentials and raw auth state.

## Grok: blocked by Provider quota

Session `sess-1790783866446` used Grok 4.7 with high reasoning. It verified normal
SSH/sudo authority and read the remote source and guidance. One read exceeded
the tool's 15-second foreground window and moved to background; the agent then
repeated that read. This is retained as an observed delay, not zero-stall proof.

The Provider then returned HTTP 429 with
`subscription:free-usage-exhausted`: the rolling 24-hour allowance reported
510172 tokens against a 500000-token limit. The session became `crashed` and
the pending ACP prompt correctly returned JSON-RPC error -32603 with the quota
diagnostic. An initial operator suspicion that the CLI hung was disproved by
the complete RPC result and process exit. No Cowboy fix or repeated Provider
request was justified by that evidence.

Grok did not implement or commit either task and did not complete the frozen
test baseline. Its complete development acceptance remains blocked until the
authorized Provider account has usable quota. Restoring quota uses the normal
Provider account/product flow, never copied credentials or live database edits.

## Connection observation

A separate OVH-owned observer kept one ordinary SSH connection to Hawk for
600.43 seconds. All 121 numbered samples arrived in order, without duplicates;
the maximum delivery interval was 5.0048 seconds for a five-second sample period,
and SSH exited zero. This does not measure new-connection latency, prove JMS
selection or establish fault recovery. The first observer script failed before
starting SSH because of an operator quoting error; staging was also retried
outside sticky `/tmp`. Neither failed setup contributes healthy observation time.

OVH's kernel reported no OOM messages in this acceptance interval. At postflight,
1203 MiB was used and 10468 MiB was available; a resource peak was not measured.
Native SFTP transferred the fixture README with an independently matched hash.
The observer and disposable worktrees are removed after verified Git archival;
the permanent Machine, SSH identity and replayable Cowboy sessions remain.

## Security and remaining scope

OVH rejects password, keyboard-interactive and root SSH authentication. Its
three operator public keys still passed fresh login and sudo from Hawk, Falcon
and MBA. No-credential and unpinned-host attempts were rejected; an unrelated
Unix user could not read the execution private key. These bounded checks are
not proof of absence of vulnerabilities. Public SSH network source admission
remains pending; key authentication restricts identity, not network location.

This run does not activate Falcon's execution cohort, finish the bidirectional
JMS fault/fail-closed gate, establish a 30-minute zero-failure window, or replace
browser Code-surface, reboot and physical-device regression acceptance. The
permanent OVH Machine, provider identities and existing sessions are retained.

## Delivery validation

Columbus passed `nix develop -c just verify`, native diff review, the clean
committed Hawk system build and the host-owned activation transaction. Cowboy
passed `nix develop -c just install` and the complete `just check-compact` gate,
including isolated PostgreSQL tests and the release build. The acceptance-only
diff received a complete evidence review: frozen hashes, actual SSH exits,
error response, patch replay, links and JSON parsing. No Cowboy component is
deployed for these documentation artifacts.
