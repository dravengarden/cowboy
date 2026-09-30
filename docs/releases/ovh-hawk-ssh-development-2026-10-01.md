# OVH Codex remote development acceptance

Codex `gpt-6-luna`, effort `max`, completed a feature and a separate storage bug
fix through ordinary `ssh hawk` from the retained production Cowboy Machine
`ovh`. All 51 unique agent tool calls used that SSH interface. The work ran in
the disposable remote project `/workspace/taskboard-acceptance`; no production
business files were modified. This is actual CLI/ACP session execution, not a
native CLI substituted for a Cowboy-managed session or a Web UI acceptance.

The [structured receipt](../acceptance-results/ovh-hawk-ssh-development-2026-10-01.json)
contains the session, commits, immutable test hashes, SSH counts and network
samples. The [tool record](../acceptance-results/ovh-hawk-ssh-development-2026-10-01/tools.json)
retains actual commands and exit codes, deduplicated by tool-call ID across
session replay. No private key, cookie, provider credential or model reasoning
is included.
The [observed test runs](../acceptance-results/ovh-hawk-ssh-development-2026-10-01/test-runs.json)
retain both baseline and intermediate failures; the
[independent results](../acceptance-results/ovh-hawk-ssh-development-2026-10-01/independent-results.txt)
retain the final suite and holdout output.

## Production identity and scope

- Machine: `ovh`; Service: `svc-4e4d5154f3df9aa109d7d841dd925fd7`.
- Session: `sess-1790783866445`, created and resumed through official
  authenticated `cowboy serve-acp` with the existing approved device client.
- The permanent Matrix SSH identity was used. There was no temporary execution
  key, alternate target address, test CA, shortened Service directory or
  per-session SSH launcher.
- [Columbus's deployed host boundary](https://github.com/dravengarden/columbus/blob/main/machines/ovh/docs/development-acceptance-2026-10-01.md)
  separates OVH operator sudo from the Cowboy user, and admits general
  development commands in an isolated Hawk workspace. Host administration,
  private operator data, local business services and forwarding remain denied.
  Namespace UID zero is not host root. Disk quotas are not supplied by that
  change; the documented memory, process and CPU limits were verified.
- Agents require logical names and paths, not physical location or proxy
  knowledge. This is transparent connectivity, not a guarantee against an
  agent deliberately inferring geography from other information.

No Cowboy Web, Controller, Machine or Plugin bytes changed for this acceptance.
Existing production Machine identity and active sessions were retained.

## Feature task

Remote commit `716c280b9a756b90d5f2c719e5b3e1c61de1ac2a` implements
`export --format csv [--tag TAG]`. Two original failures became five passing
tests. Coverage includes Unicode, commas, quotes, embedded newlines, numeric
ID ordering, lowercase booleans, unique sorted tags, filtering, LF output and
read-only behavior. The original storage implementation remained unchanged.

An independent holdout verified 100 complex CSV records. The original
`test_existing.py` and `test_export.py` bytes matched their frozen hashes.
This round used 24 SSH tool calls and ended normally through ACP.

Intermediate failures are retained: `rg` was absent and the agent used ordinary
shell tools; the host's follow-up package correction supplies it and restores
the standard hostname/process tools. Its added CSV regression initially reversed the expected column
values, then was corrected. These were not network failures. The fixed
acceptance tests were not weakened.

## Storage bug task

After a separate operator commit added frozen storage regressions, the same
Cowboy session resumed and reproduced three failures: concurrent mutation
loss/failure, corruption of the old JSON under `RLIMIT_FSIZE`, and mode `0644`
on a newly created database. Remote commit
`da81fa00436d92b3d6c1eef97fc332f0a1222e21` uses a stable sidecar `flock` around
load/update/save, same-directory temporary files, fsync and atomic replacement.
It preserves existing permissions and creates private new databases.

The agent's first edit had a missing closing parenthesis and a missing import
in its new regression module; tests caught both before the final commit. After
correction, all 11 tests passed. An independent operator reran the full suite
through the permanent SSH identity and obtained the same result. The three
frozen test files were byte-for-byte unchanged.

The independent [holdout](../acceptance-results/ovh-hawk-ssh-development-2026-10-01/holdout.py)
also passed:

- 144 concurrent writers retained every task and unique increasing IDs;
- 265 concurrent reads observed complete JSON;
- 12 interrupted-writer trials preserved a complete old or committed new state;
- mode `0640` remained unchanged;
- corrupt input was rejected without reset;
- export worked in a read-only directory without creating files;
- the 100-record CSV feature regression still passed.

This round used 27 unique SSH tool calls. The observer's 900-second whole-turn
wait expired after the commit; the underlying session was not canceled or
resubmitted. A fresh official ACP load/observation recovered the replay and
reported `turnRunning=false`, `agentAlive=true`. The timeout is retained as an
observer limitation, not hidden as an ordinary successful prompt response.

## Network evidence and limits

An OVH-owned observer, independent of the operator's bootstrap connection,
received 61 consecutive five-second SSH samples, exit zero, maximum arrival
gap 5.0063 seconds. The stream survived the execution-user manager mask/stop.
It had no missing or duplicate sequence numbers. The earlier nested
operator-to-OVH-to-Hawk observer failed after two samples when the outer SSH
connection closed; both records remain in the receipt.

This does not establish a thirty-minute zero-failure window, JMS fault
selection/recovery budgets, fail-closed behavior, or bidirectional outer-path
proof. Falcon's execution cohort, restart/cold recovery, browser Code surface
and physical mobile-device checks remain outstanding. No transport policy was
changed to make these development tasks pass.

## Reproduce without business data

The complete four-commit fixture history is retained as a textual
[Git patch series](../acceptance-results/ovh-hawk-ssh-development-2026-10-01/fixture-history.patch).
From an authorized workspace with the managed `ssh hawk` interface, restore it
only into an absent disposable directory:

```sh
ssh hawk 'mkdir /workspace/taskboard-acceptance && cd /workspace/taskboard-acceptance && git init && git -c user.name="Matrix acceptance" -c user.email=matrix-acceptance@localhost am' < fixture-history.patch
ssh hawk 'cd /workspace/taskboard-acceptance && python3 -m unittest -v'
ssh hawk python3 - < holdout.py
```

The private operator archive additionally holds a verified complete Git bundle
with the original commit IDs. Applying the textual patch series reproduced the
exact final Git tree `50a2124a0962aa55c0cd460a9441f3eba641268c`. The production
test directory and observer were removed after archiving; the OVH Machine,
permanent SSH identity and persistent development workspace remain in place.
